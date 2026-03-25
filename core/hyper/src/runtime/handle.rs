use async_trait::async_trait;

use crate::error::HyperResult;

/// Runtime handle trait (Native / WASM)
///
/// Hyper manages in-process runtime lifecycle through this interface.
/// Native: node runtime compiled into the same binary, directly implements this trait.
/// WASM: native shell wraps a WASM instance and implements this trait.
#[async_trait]
pub trait ActrSystemHandle: Send + Sync {
    /// Start the runtime
    async fn start(&self) -> HyperResult<()>;

    /// Gracefully shut down the runtime, wait for in-flight messages to complete
    async fn shutdown(&self) -> HyperResult<()>;

    /// Whether healthy (used for Hyper-side monitoring)
    fn is_healthy(&self) -> bool;

    /// Runtime unique identifier (for debugging)
    fn id(&self) -> &str;
}

/// WASM instance handle
///
/// Created and held by the native shell runtime.
/// On hot update, the old instance is unloaded and a new instance handle is created.
#[derive(Debug)]
pub struct WasmInstanceHandle {
    /// WASM instance unique ID (generated on each load)
    pub instance_id: String,
    /// Corresponding ActrType
    pub actr_type: String,
}

impl WasmInstanceHandle {
    pub fn new(instance_id: impl Into<String>, actr_type: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.into(),
            actr_type: actr_type.into(),
        }
    }
}
