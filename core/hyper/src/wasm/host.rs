//! WasmHost — Wasmtime Component Model host engine.
//!
//! Drives wasm workloads packaged as Component Model components. A single
//! [`WasmHost`] compiles a component once; Hyper can then derive multiple
//! internal runtime instances from that compilation, one per logical actor.
//!
//! # Contract
//!
//! The guest component must implement the `actr:workload@0.2.0`
//! `actr-workload-guest-v2` world defined in
//! `core/framework/wit-v2/actr-workload.wit`. That contract carries one
//! `dispatch(envelope, ctx)` export for inbound RPC plus sixteen observation
//! hooks (lifecycle + signaling + transport + credential + mailbox),
//! exactly mirroring [`actr_framework::Workload`]. Every host import and
//! every workload export is a real WIT `async func`, which unlocks
//! wasmtime 46's Component Model Concurrency: multiple `dispatch` calls may
//! be in flight on one instance, cooperatively interleaving at each
//! host-import `.await`.
//!
//! Host imports (`call`, `tell`, `call-raw`, `discover`, `log-message`) are
//! serviced via the caller-supplied [`HostAbiFn`] bridge threaded into the
//! WASM [`Store`] under a per-invocation `ctx-token`.
//!
//! # Async model
//!
//! Same-instance concurrency is driven through `Store::run_concurrent`; the
//! Accessor-based host imports recover the per-call `HostAbiFn` by the
//! `ctx-token` threaded into the guest's `invocation-ctx`. The 0.1.0
//! synchronous world is retired — a component exporting it is rejected at
//! load with an actionable rebuild hint. See [`super::host_v2::WasmWorkloadV2`]
//! for the execution path.

use std::collections::HashMap;
use std::sync::Arc;

use wasmtime::component::{Component, ResourceTable};
use wasmtime::{Config, Engine, OptLevel, RegallocAlgorithm};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::config::WasmRuntimeLimits;
use crate::workload::{HostAbiFn, InvocationContext};

use super::host_v2::WasmWorkloadV2;
use super::runtime_limits::{
    EpochTicker, StoreResourceLimiter, acquire_compile, acquire_store, record_compile_failure,
    record_resource_denial,
};
use crate::wasm::error::{WasmError, WasmResult};

// ─────────────────────────────────────────────────────────────────────────────
// Engine configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Build a wasmtime [`Config`] enabling the Component Model async path.
///
/// We pin `wasm_component_model_async(true)` and `concurrency_support(true)` —
/// the wasmtime 46 gates that govern the component-model async + concurrency
/// surface. When `concurrency_support` is off, `Store::run_concurrent`
/// panics; the V2 async world depends on it, so we set it explicitly to be
/// self-documenting and to guard against an upstream default flip.
/// Enabling it alongside `wasm_component_model_async(true)` is the supported
/// combination.
fn build_engine(limits: &WasmRuntimeLimits) -> WasmResult<Engine> {
    let mut config = Config::new();
    // `async_support(true)` was required before wasmtime 43; since then
    // the `async` Cargo feature alone enables async at the engine level.
    // We pair it with explicit component-model flags to be self-documenting.
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);
    // Explicitly gate the concurrency surface (`run_concurrent`) on. Defaults
    // to true on wasmtime 46; pinned here so a future default flip can never
    // silently turn the V2 concurrent runner into a runtime panic.
    config.concurrency_support(true);
    // issue #346: fuel + epoch preempt non-yielding compute (a pure-compute
    // infinite loop never awaits a host import, so DispatchConcurrency's
    // dispatch_timeout cannot interrupt it — fuel/epoch insert check points
    // into the compiled wasm). Stack caps guard against stack-overflow
    // recursion. `fuel_async_yield_interval = None` traps on exhaustion; the
    // instance is rebuilt on the next entry (`WasmWorkloadV2::ensure_instance`).
    config.consume_fuel(true);
    config.epoch_interruption(true);
    config.max_wasm_stack(limits.max_wasm_stack);
    config.async_stack_size(limits.async_stack_size);
    // NOTE: `fuel_async_yield_interval` is a `Store` method in wasmtime 46
    // (not `Config`); set per-store in `instantiate_parts_v2`.
    if std::env::var_os("ACTR_WASM_FAST_COMPILE").is_some() {
        config.cranelift_opt_level(OptLevel::None);
        config.cranelift_regalloc_algorithm(RegallocAlgorithm::SinglePass);
    }
    Engine::new(&config)
        .map_err(|e| WasmError::LoadFailed(format!("wasmtime engine construction failed: {e}")))
}

// ─────────────────────────────────────────────────────────────────────────────
// HostState — per-Store runtime state
// ─────────────────────────────────────────────────────────────────────────────

/// Per-instance host state threaded through the wasmtime [`Store`].
///
/// Holds:
/// - the WASI p2 context + resource table (required for any component
///   that transitively imports WASI interfaces);
/// - a per-invocation table keyed by `ctx-token`, holding the
///   [`InvocationContext`] + [`HostAbiFn`] for every in-flight dispatch on
///   the 0.2.0 async world — installed before `call_dispatch` and retired
///   after, so the Accessor-based host imports recover the right per-call
///   state without a shared single slot that would cross-talk;
/// - stream callback token tracking, so a callback registered during one
///   invocation but invoked later resolves back to a live token.
/// - issue #346 per-store resource limits (memory/table/instance caps).
pub(crate) struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    /// Per-invocation table keyed by `ctx-token`, used by the V2 (0.2.0
    /// async world) Accessor-based host imports. Each in-flight invocation
    /// keys its own `HostAbiFn` / `InvocationContext` by its token, so the
    /// static Accessor import methods recover the correct one without a
    /// shared single slot that would cross-talk.
    invocations: HashMap<u64, InvocationEntry>,
    /// V2 stream callbacks are registered during one invocation but execute
    /// later, after that invocation's token has been retired. Guest callbacks
    /// commonly capture the registering `WasmContext`, so remember which token
    /// that context carries for each stream. While a DataChunk callback is
    /// active, `stream_token_aliases` temporarily resolves the captured token
    /// to the callback invocation's live token/HostAbiFn.
    stream_context_tokens: HashMap<String, u64>,
    stream_token_aliases: HashMap<u64, u64>,
    /// Monotonic token allocator. Reset to zero whenever the map is cleared
    /// on a trap-poison so a rebuilt instance starts from a clean sheet.
    next_token: u64,
    /// issue #346: per-store resource limits (memory/table/instance caps).
    /// Installed via `Store::limiter` so `memory.grow`/`table.grow` over the
    /// bound trap (or return an error, per `trap_on_grow_failure`).
    pub(crate) limits: StoreResourceLimiter,
}

/// One live invocation's host-facing state, keyed by `ctx-token` in
/// [`HostState::invocations`].
pub(crate) struct InvocationEntry {
    #[allow(dead_code)]
    pub(crate) ctx: InvocationContext,
    pub(crate) host_abi: HostAbiFn,
}

impl std::fmt::Debug for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("invocations", &self.invocations.len())
            .field("stream_context_tokens", &self.stream_context_tokens.len())
            .field("stream_token_aliases", &self.stream_token_aliases.len())
            .field("next_token", &self.next_token)
            .finish_non_exhaustive()
    }
}

impl HostState {
    pub(crate) fn new(limits: &WasmRuntimeLimits) -> Self {
        Self {
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
            table: ResourceTable::new(),
            invocations: HashMap::new(),
            stream_context_tokens: HashMap::new(),
            stream_token_aliases: HashMap::new(),
            next_token: 0,
            limits: StoreResourceLimiter::new(limits),
        }
    }

    /// Allocate a fresh `ctx-token` and register a live invocation's
    /// `InvocationContext` + `HostAbiFn` under it. Returns the token to
    /// thread into the guest's `invocation-ctx` (V2 world). Tokens are
    /// monotonic within a Store's lifetime.
    pub(crate) fn alloc_invocation(&mut self, ctx: InvocationContext, host_abi: HostAbiFn) -> u64 {
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);
        self.invocations
            .insert(token, InvocationEntry { ctx, host_abi });
        token
    }

    /// Clone the `HostAbiFn` registered for `token`, if any. `HostAbiFn` is
    /// an `Arc`, so the clone is a refcount bump safe to carry across an
    /// `.await` inside an Accessor host method (the store borrow is not
    /// held across the await).
    pub(crate) fn invocation_host_abi(&self, token: u64) -> Option<HostAbiFn> {
        let token = self
            .stream_token_aliases
            .get(&token)
            .copied()
            .unwrap_or(token);
        self.invocations
            .get(&token)
            .map(|e| Arc::clone(&e.host_abi))
    }

    /// Associate a successfully registered stream with the token embedded in
    /// the guest callback's captured `WasmContext`.
    pub(crate) fn register_stream_context(&mut self, stream_id: String, token: u64) {
        self.stream_context_tokens.insert(stream_id, token);
    }

    /// Forget the captured context associated with an unregistered stream.
    pub(crate) fn unregister_stream_context(&mut self, stream_id: &str) {
        self.stream_context_tokens.remove(stream_id);
    }

    /// Temporarily route a registered callback's captured token through the
    /// currently-active DataChunk invocation. DataChunk commands are runner
    /// barriers, so at most one alias for a stream callback is active.
    pub(crate) fn begin_stream_callback(
        &mut self,
        stream_id: &str,
        invocation_token: u64,
    ) -> Option<u64> {
        let captured_token = self.stream_context_tokens.get(stream_id).copied()?;
        if captured_token != invocation_token {
            self.stream_token_aliases
                .insert(captured_token, invocation_token);
        }
        Some(captured_token)
    }

    /// Remove the temporary alias installed by [`Self::begin_stream_callback`].
    pub(crate) fn end_stream_callback(
        &mut self,
        captured_token: Option<u64>,
        invocation_token: u64,
    ) {
        let Some(captured_token) = captured_token else {
            return;
        };
        if captured_token != invocation_token
            && self.stream_token_aliases.get(&captured_token) == Some(&invocation_token)
        {
            self.stream_token_aliases.remove(&captured_token);
        }
    }

    /// Retire the invocation registered for `token` once its guest call has
    /// completed (success or business error). No-op if already gone.
    pub(crate) fn remove_invocation(&mut self, token: u64) {
        self.invocations.remove(&token);
    }

    pub(crate) fn invocation_count(&self) -> usize {
        self.invocations.len()
    }

    /// Drop every live invocation and reset the token counter. Called when a
    /// trap poisons the store: the whole in-flight set is dead, so the
    /// rebuilt instance starts clean.
    pub(crate) fn clear_invocations(&mut self) {
        self.invocations.clear();
        self.stream_context_tokens.clear();
        self.stream_token_aliases.clear();
        self.next_token = 0;
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WasmHost
// ─────────────────────────────────────────────────────────────────────────────

/// Compiled wasm Component engine.
///
/// One `WasmHost` corresponds to one compiled component. Hyper uses it
/// internally to instantiate one runtime workload per actor instance.
pub struct WasmHost {
    engine: Engine,
    component: Component,
    limits: WasmRuntimeLimits,
    epoch_ticker: Arc<EpochTicker>,
}

impl std::fmt::Debug for WasmHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHost").finish_non_exhaustive()
    }
}

impl WasmHost {
    /// Compile a Component from raw bytes.
    ///
    /// CPU-intensive; callers should run this on a blocking task.
    /// Errors include non-Component inputs (e.g. a legacy core wasm
    /// module saved in a pre-Phase-1 `.actr` package) — callers get a
    /// clear `LoadFailed` in that case and should surface the migration
    /// guidance up to the `.actr` loader.
    ///
    /// A second class of legacy input is a package built by an old SDK
    /// (wit-bindgen <= 0.57 with the async-lift ABI). wasmtime 46 rejects
    /// the `async` canonical option on a synchronous WIT function type, so
    /// those binaries fail here; we map that to an actionable rebuild hint.
    pub fn compile(wasm_bytes: &[u8]) -> WasmResult<Self> {
        Self::compile_with_limits(wasm_bytes, &WasmRuntimeLimits::default())
    }

    /// Compile with explicit resource limits (issue #346). Production callers
    /// thread the configured [`WasmRuntimeLimits`]; tests use [`compile`].
    pub fn compile_with_limits(wasm_bytes: &[u8], limits: &WasmRuntimeLimits) -> WasmResult<Self> {
        limits.validate().map_err(WasmError::VerificationFailed)?;
        if wasm_bytes.len() > limits.max_component_bytes {
            record_resource_denial();
            return Err(WasmError::ResourceLimitExceeded("component byte size"));
        }
        let _compile_permit = acquire_compile(limits)?;
        let engine = build_engine(limits)?;
        let epoch_ticker = EpochTicker::spawn(&engine, limits.epoch_tick)?;
        let component = Component::from_binary(&engine, wasm_bytes).map_err(|e| {
            record_compile_failure();
            let raw = format!("{e:#}");
            if raw.contains("`async` canonical option requires an async function type") {
                return WasmError::LoadFailed(format!(
                    "this .actr package was built by an old SDK (wit-bindgen \
                     <= 0.57 async-lift ABI), which wasmtime 46 rejects per \
                     the Component Model spec. Rebuild it with the current SDK \
                     (synchronous-lift packages run on both new and old hosts). \
                     wasmtime reported: {raw}"
                ));
            }
            WasmError::LoadFailed(format!(
                "wasm bytes did not load as a Component (this host \
                 requires Component Model binaries as of .actr format \
                 bump; wasmtime reported: {raw})"
            ))
        })?;
        tracing::info!(wasm_bytes = wasm_bytes.len(), "wasm Component compiled");
        Ok(Self {
            engine,
            component,
            limits: *limits,
            epoch_ticker,
        })
    }

    /// Instantiate the component into a runnable internal workload.
    ///
    /// Probes the component's exported world and rejects anything that is not
    /// the sole supported `actr:workload@0.2.0` async world, then instantiates
    /// it on the `run_concurrent` async path via [`WasmWorkloadV2::instantiate`].
    /// A retired `@0.1.0` synchronous-world package (or an unrecognised world)
    /// is rejected with an actionable rebuild hint rather than loaded.
    ///
    /// Builds a fresh [`Linker`] per instance (cheap) inside the V2
    /// instantiation helper, registers WASI p2 as well as the generated
    /// `actr:workload/host@0.2.0` linker, and runs `Component::instantiate_async`.
    pub(crate) async fn instantiate(&self) -> WasmResult<WasmWorkloadV2> {
        let store_permit = acquire_store(&self.limits)?;
        probe_world(&self.component, &self.engine)?;
        WasmWorkloadV2::instantiate(
            &self.engine,
            &self.component,
            &self.limits,
            Arc::clone(&self.epoch_ticker),
            store_permit,
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// World probing
// ─────────────────────────────────────────────────────────────────────────────

/// Reject a compiled [`Component`] unless it implements the sole supported
/// `actr:workload@0.2.0` async world. The decision is grounded purely in the
/// component's exported instance types — never reads the `.actr` manifest —
/// so it reflects what the binary actually exports.
///
/// A component exporting the retired `actr:workload@0.1.0` synchronous world
/// (or no recognised world at all) is rejected with an actionable rebuild
/// hint: V1 packages must be rebuilt with the current SDK, which now emits
/// only the V2 async-world ABI.
fn probe_world(component: &Component, engine: &Engine) -> WasmResult<()> {
    let mut saw_v2 = false;
    let mut saw_v1 = false;
    for (name, _item) in component.component_type().exports(engine) {
        if name.starts_with("actr:workload/workload@0.2.0") {
            saw_v2 = true;
        } else if name.starts_with("actr:workload/workload@0.1.0") {
            saw_v1 = true;
        }
    }
    match (saw_v1, saw_v2) {
        (false, true) => Ok(()),
        (true, false) => Err(WasmError::LoadFailed(
            "component exports the retired actr:workload@0.1.0 synchronous world; \
             rebuild the package with the current SDK, which emits only the \
             actr:workload@0.2.0 async-world ABI"
                .to_string(),
        )),
        (true, true) => Err(WasmError::LoadFailed(
            "component exports both actr:workload/workload@0.1.0 and @0.2.0; \
             a package must implement exactly one world (the 0.1.0 synchronous \
             world is retired — rebuild with the current SDK, which emits only \
             the @0.2.0 async-world ABI)"
                .to_string(),
        )),
        (false, false) => Err(WasmError::LoadFailed(
            "component exports no recognised actr:workload/workload world \
             (expected @0.2.0); rebuild the package with a current SDK"
                .to_string(),
        )),
    }
}

/// Convert the invocation timeout into an epoch deadline (tick count).
/// Rounded up so the deadline is never shorter than the timeout. The epoch
/// interrupt is a backstop for fuel: it fires even if a tight loop burns no
/// fuel in a single Wasm op, provided a background ticker is advancing the
/// engine epoch (`Store::set_epoch_deadline` is relative to the engine epoch
/// at call time).
pub(crate) fn epoch_deadline_ticks(limits: &WasmRuntimeLimits) -> u64 {
    let tick_nanos = limits.epoch_tick.as_nanos().max(1);
    u64::try_from(limits.invocation_timeout.as_nanos() / tick_nanos)
        .unwrap_or(u64::MAX)
        .saturating_add(1)
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
