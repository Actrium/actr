//! Phase 1 follow-up: Component Model guest bridge for the Web runtime.
//!
//! # Status: placeholder / migration sketch
//!
//! This module intentionally contains no runnable code. It exists to hold
//! the design notes and API shape for the planned browser-side Component
//! Model bridge so the follow-up work lands with a clear target, and so
//! readers of [`guest_bridge`](super::guest_bridge) can trace where the
//! legacy ABI exit ramp leads.
//!
//! The native host (`core/hyper`) is already on the Component Model as of
//! Phase 1 Commits 1–4. The browser runtime cannot yet embed a full
//! Component Model interpreter, so the browser path takes a different
//! tack:
//!
//! 1. The user builds their guest with the same `entry!` macro, targeting
//!    `wasm32-wasip2`. The resulting artifact is a Component (`.wasm` in
//!    `application/wasm` that adverts the Component Model custom section).
//! 2. A build-time `jco transpile <component>.wasm` step turns that
//!    Component into a pair: an ES module implementing the canonical ABI
//!    adapter, plus the original core wasm module inside.
//! 3. The SW runtime calls into the ES module through wasm-bindgen
//!    instead of dispatching protobuf `AbiFrame` envelopes.
//!
//! # Planned public surface
//!
//! ```ignore
//! // Replaces `register_guest_workload` for Component guests.
//! #[wasm_bindgen]
//! pub fn register_component_workload(bindings: js_sys::Object);
//! ```
//!
//! `bindings` is the object returned by the jco-transpiled module's
//! default export (the `actr-workload-guest` world binding). It exposes
//! the sixteen observation hooks plus `dispatch` as async JS functions,
//! and it accepts a host-imports object that runtime-sw constructs to
//! service `call` / `tell` / `call-raw` / `discover` / `log-message` / the
//! three context getters defined in
//! `core/framework/wit/actr-workload.wit`.
//!
//! # Dispatch flow
//!
//! ```text
//! inbound RPC ──► ServiceHandlerFn (legacy, same shape)
//!                 │
//!                 ├─ set GUEST_CTX
//!                 ├─ build WIT `rpc-envelope` JS object
//!                 ├─ await bindings.workload.dispatch(envelope, hostImports)
//!                 │     │
//!                 │     └─ host imports bounce back to Rust via
//!                 │        `component_host_impl` (mirrors `host.rs`).
//!                 ├─ clear GUEST_CTX
//!                 └─ marshal result back to the caller as Vec<u8>
//! ```
//!
//! # Outstanding tasks before this module has real code
//!
//! - `bindings/web/scripts/build-wasm.sh` needs a `jco transpile` step,
//!   which means jco must be installable inside CI (npm dependency or
//!   devDependency on `@bytecodealliance/jco`).
//! - `bindings/web/examples/echo/{client-guest,server-guest}/Cargo.toml`
//!   need their `crate-type` changed from `cdylib` to Component output,
//!   and their build target flipped to `wasm32-wasip2`.
//! - `wasm-component-ld 0.5.22` needs to be installable in the Web build
//!   environment (already pinned for the native/CI path — see Commit 7).
//! - `actr_framework::guest::abi` items consumed only by the legacy Web
//!   bridge (`AbiFrame`, `GuestHandleV1`, `HostCallV1`, `HostTellV1`,
//!   `HostCallRawV1`, `HostDiscoverV1`, `op::*`) can be removed once the
//!   cdylib path has also migrated.
//!
//! None of the above are started in Commit 5. This module is a map, not
//! a trail.
