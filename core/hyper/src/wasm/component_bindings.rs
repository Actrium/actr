//! Component Model bindings for the `actr:workload@0.1.0` WIT contract.
//!
//! Phase 1, Commit 1: these bindings live **alongside** the legacy
//! handwritten `host.rs` / `abi.rs` ABI path rather than replacing it.
//! Commit 2 of Phase 1 rewrites `host.rs` to drive Component Model
//! instances through these bindings and deletes the legacy path.
//!
//! # Async shape
//!
//! Host-side imports are generated as `async fn` via
//! `imports: { default: async | trappable }` and guest-exports as `async
//! fn` via `exports: { default: async }`. The underlying WIT is plain
//! sync `func` — this is the Phase-0.5-validated combination that keeps
//! actr's single-threaded-actor invariant while still driving real I/O
//! through tokio.
//!
//! # Why `async | trappable`
//!
//! `async` makes every import an `async fn`; `trappable` lets host
//! implementations return a `Result<T, wasmtime::Error>` so that host-side
//! failures (e.g. a downstream RPC timeout) cleanly surface as `Result`
//! rather than forcing a `Trap`. The generated return shape is
//! `wasmtime::Result<Result<T, actr-error>>`: the outer `Result` signals
//! trap-level failure, the inner `Result` is the WIT variant return.
//!
//! # Why no `with` map yet
//!
//! The spike uses no `with: { ... }` remapping because the generated
//! types (Rust structs mirroring the WIT records) are exactly what we
//! want for Commit 2's `HostState` implementation. When Commit 2 wires
//! these up to `actr_framework` types, we'll decide per-type whether to
//! remap (e.g. `actr:workload/types/actr-id` → `actr_protocol::ActrId`)
//! or to translate at the boundary inside the `Host` impl. The spike
//! prefers the latter for now — boundary translation keeps the generated
//! bindings self-contained and makes the mapping rules reviewable.

wasmtime::component::bindgen!({
    world: "actr-workload-guest",
    path: "../framework/wit",
    imports: { default: async | trappable },
    exports: { default: async },
});
