pub mod handle;
pub mod monitor;
pub mod spawn;

pub use handle::{ActrSystemHandle, ChildProcessHandle, ChildProcessState, WasmInstanceHandle};
pub use monitor::monitor_process;
pub use spawn::{RestartPolicy, SpawnConfig};

use std::sync::Arc;

/// Hyper's internal representation of the managed ActrSystem+Workload stack
///
/// Hyper's involvement depth differs across the three modes:
/// - Native: in-process, holds ActrSystem handle
/// - Process: independent child process, lifecycle management only (no message proxying)
/// - Wasm: ActrSystem native shell holds the WASM engine (WasmEngine is an ActrSystem internal trait)
pub enum ActorRuntime {
    /// Mode 1 — Native
    ///
    /// ActrSystem+Workload compiled into the same binary (or FFI statically linked), runs as coroutines.
    /// Hyper directly holds the ActrSystem handle, manages lifecycle via `ActrSystemHandle` trait.
    Native(Arc<dyn ActrSystemHandle>),

    /// Mode 2 — Process
    ///
    /// ActrSystem+Workload runs as an independent OS process.
    /// Hyper completes signature verification + AIS registration, then spawns a child process;
    /// credential is passed via environment variables.
    /// The ActrSystem inside the child process connects directly to signaling;
    /// message traffic does not go through Hyper.
    /// Hyper only manages lifecycle: health check, restart policy.
    Process(Box<ChildProcessHandle>),

    /// Mode 3 — WASM
    ///
    /// ActrSystem+Workload compiled as .wasm, loaded and executed by ActrSystem native shell.
    /// WASM engine is an ActrSystem internal concern; Hyper is unaware of the specific engine implementation.
    /// The ActrSystem inside WASM accesses external capabilities (storage, crypto, network I/O)
    /// through Hyper host functions.
    Wasm(WasmInstanceHandle),
}

impl ActorRuntime {
    /// ActrType string (for logging/debugging)
    pub fn actr_type(&self) -> &str {
        match self {
            ActorRuntime::Native(h) => h.id(),
            ActorRuntime::Process(h) => &h.actr_type,
            ActorRuntime::Wasm(h) => &h.actr_type,
        }
    }

    /// Whether healthy
    pub fn is_healthy(&self) -> bool {
        match self {
            ActorRuntime::Native(h) => h.is_healthy(),
            ActorRuntime::Process(h) => h.is_running(),
            ActorRuntime::Wasm(_) => true, // WASM health status maintained by ActrSystem shell
        }
    }

    /// Runtime mode name (for logging)
    pub fn mode_name(&self) -> &'static str {
        match self {
            ActorRuntime::Native(_) => "native",
            ActorRuntime::Process(_) => "process",
            ActorRuntime::Wasm(_) => "wasm",
        }
    }
}
