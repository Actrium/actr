use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wasmtime::Engine;

use crate::config::WasmRuntimeLimits;

use super::error::{WasmError, WasmResult};

static ACTIVE_COMPILES: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_INSTANTIATES: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_STORES: AtomicUsize = AtomicUsize::new(0);
static RESERVED_LINEAR_MEMORY: AtomicUsize = AtomicUsize::new(0);
static OUTSTANDING_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);

static DENIED_COMPILES: AtomicU64 = AtomicU64::new(0);
static DENIED_INSTANTIATES: AtomicU64 = AtomicU64::new(0);
static DENIED_STORES: AtomicU64 = AtomicU64::new(0);
static DENIED_INVOCATIONS: AtomicU64 = AtomicU64::new(0);
static OUT_OF_FUEL_TRAPS: AtomicU64 = AtomicU64::new(0);
static EPOCH_TRAPS: AtomicU64 = AtomicU64::new(0);
static INVOCATION_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static RESOURCE_DENIALS: AtomicU64 = AtomicU64::new(0);
static COMPILE_FAILURES: AtomicU64 = AtomicU64::new(0);
static INSTANTIATE_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Process-wide WASM resource counters suitable for metrics exporters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmRuntimeStats {
    pub active_compiles: usize,
    pub active_instantiates: usize,
    pub active_stores: usize,
    pub reserved_linear_memory: usize,
    pub outstanding_invocations: usize,
    pub denied_compiles: u64,
    pub denied_instantiates: u64,
    pub denied_stores: u64,
    pub denied_invocations: u64,
    pub out_of_fuel_traps: u64,
    pub epoch_traps: u64,
    pub invocation_timeouts: u64,
    pub resource_denials: u64,
    pub compile_failures: u64,
    pub instantiate_failures: u64,
}

pub fn wasm_runtime_stats() -> WasmRuntimeStats {
    WasmRuntimeStats {
        active_compiles: ACTIVE_COMPILES.load(Ordering::Relaxed),
        active_instantiates: ACTIVE_INSTANTIATES.load(Ordering::Relaxed),
        active_stores: ACTIVE_STORES.load(Ordering::Relaxed),
        reserved_linear_memory: RESERVED_LINEAR_MEMORY.load(Ordering::Relaxed),
        outstanding_invocations: OUTSTANDING_INVOCATIONS.load(Ordering::Relaxed),
        denied_compiles: DENIED_COMPILES.load(Ordering::Relaxed),
        denied_instantiates: DENIED_INSTANTIATES.load(Ordering::Relaxed),
        denied_stores: DENIED_STORES.load(Ordering::Relaxed),
        denied_invocations: DENIED_INVOCATIONS.load(Ordering::Relaxed),
        out_of_fuel_traps: OUT_OF_FUEL_TRAPS.load(Ordering::Relaxed),
        epoch_traps: EPOCH_TRAPS.load(Ordering::Relaxed),
        invocation_timeouts: INVOCATION_TIMEOUTS.load(Ordering::Relaxed),
        resource_denials: RESOURCE_DENIALS.load(Ordering::Relaxed),
        compile_failures: COMPILE_FAILURES.load(Ordering::Relaxed),
        instantiate_failures: INSTANTIATE_FAILURES.load(Ordering::Relaxed),
    }
}

#[derive(Debug, Clone, Copy)]
enum CounterKind {
    Compile,
    Instantiate,
    Store,
    Memory,
    Invocation,
}

#[derive(Debug)]
pub(crate) struct QuotaPermit {
    kind: CounterKind,
    amount: usize,
}

impl Drop for QuotaPermit {
    fn drop(&mut self) {
        counter(self.kind).fetch_sub(self.amount, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub(crate) struct StorePermit {
    _store: QuotaPermit,
    _memory: QuotaPermit,
}

fn counter(kind: CounterKind) -> &'static AtomicUsize {
    match kind {
        CounterKind::Compile => &ACTIVE_COMPILES,
        CounterKind::Instantiate => &ACTIVE_INSTANTIATES,
        CounterKind::Store => &ACTIVE_STORES,
        CounterKind::Memory => &RESERVED_LINEAR_MEMORY,
        CounterKind::Invocation => &OUTSTANDING_INVOCATIONS,
    }
}

fn acquire(
    kind: CounterKind,
    amount: usize,
    limit: usize,
    denied: &AtomicU64,
    label: &'static str,
) -> WasmResult<QuotaPermit> {
    let result = counter(kind).fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        current.checked_add(amount).filter(|next| *next <= limit)
    });
    match result {
        Ok(_) => Ok(QuotaPermit { kind, amount }),
        Err(current) => {
            denied.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                resource = label,
                current,
                limit,
                "WASM process quota denied"
            );
            Err(WasmError::ResourceLimitExceeded(label))
        }
    }
}

pub(crate) fn acquire_compile(limits: &WasmRuntimeLimits) -> WasmResult<QuotaPermit> {
    acquire(
        CounterKind::Compile,
        1,
        limits.max_concurrent_compiles,
        &DENIED_COMPILES,
        "concurrent component compilations",
    )
}

pub(crate) fn acquire_instantiate(limits: &WasmRuntimeLimits) -> WasmResult<QuotaPermit> {
    acquire(
        CounterKind::Instantiate,
        1,
        limits.max_concurrent_instantiates,
        &DENIED_INSTANTIATES,
        "concurrent component instantiations",
    )
}

pub(crate) fn acquire_store(limits: &WasmRuntimeLimits) -> WasmResult<StorePermit> {
    let store = acquire(
        CounterKind::Store,
        1,
        limits.max_active_stores,
        &DENIED_STORES,
        "active WASM stores",
    )?;
    let memory = acquire(
        CounterKind::Memory,
        limits.max_linear_memory,
        limits.max_total_linear_memory,
        &DENIED_STORES,
        "aggregate configured linear memory",
    )?;
    Ok(StorePermit {
        _store: store,
        _memory: memory,
    })
}

pub(crate) fn acquire_invocation(limits: &WasmRuntimeLimits) -> WasmResult<QuotaPermit> {
    acquire(
        CounterKind::Invocation,
        1,
        limits.max_outstanding_invocations,
        &DENIED_INVOCATIONS,
        "outstanding WASM invocations",
    )
}

pub(crate) fn record_out_of_fuel() {
    OUT_OF_FUEL_TRAPS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_epoch_trap() {
    EPOCH_TRAPS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_timeout() {
    INVOCATION_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_resource_denial() {
    RESOURCE_DENIALS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_compile_failure() {
    COMPILE_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_instantiate_failure() {
    INSTANTIATE_FAILURES.fetch_add(1, Ordering::Relaxed);
}

/// Dedicated epoch ticker for one Engine. The native thread guarantees epoch
/// progress even when non-yielding guest code monopolizes a Tokio worker.
#[derive(Debug)]
pub(crate) struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    pub(crate) fn spawn(engine: &Engine, tick: Duration) -> WasmResult<Arc<Self>> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_engine = engine.clone();
        let handle = thread::Builder::new()
            .name("actr-wasm-epoch".to_string())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    thread::park_timeout(tick);
                    if !thread_stop.load(Ordering::Acquire) {
                        thread_engine.increment_epoch();
                    }
                }
            })
            .map_err(|e| WasmError::LoadFailed(format!("spawn WASM epoch ticker: {e}")))?;
        Ok(Arc::new(Self {
            stop,
            handle: Some(handle),
        }))
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            if handle.join().is_err() {
                tracing::error!("WASM epoch ticker thread panicked");
            }
        }
    }
}
