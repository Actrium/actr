pub mod handle;

pub use handle::{ActrSystemHandle, WasmInstanceHandle};

use std::sync::Arc;

/// Hyper's internal representation of the managed ActrSystem+Workload stack
///
/// Hyper's involvement depth differs across the execution body types:
/// - Native: in-process, holds ActrSystem handle
/// - Wasm: ActrSystem native shell holds the WASM engine (WasmEngine is an ActrSystem internal trait)
pub enum ActorRuntime {
    /// Native — source integration
    ///
    /// ActrSystem+Workload compiled into the same binary (or FFI statically linked), runs as coroutines.
    /// Hyper directly holds the ActrSystem handle, manages lifecycle via `ActrSystemHandle` trait.
    Native(Arc<dyn ActrSystemHandle>),

    /// WASM — runtime-loaded .wasm module
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
            ActorRuntime::Wasm(h) => &h.actr_type,
        }
    }

    /// Whether healthy
    pub fn is_healthy(&self) -> bool {
        match self {
            ActorRuntime::Native(h) => h.is_healthy(),
            ActorRuntime::Wasm(_) => true, // WASM health status maintained by ActrSystem shell
        }
    }

    /// Runtime mode name (for logging)
    pub fn mode_name(&self) -> &'static str {
        match self {
            ActorRuntime::Native(_) => "native",
            ActorRuntime::Wasm(_) => "wasm",
        }
    }
}
