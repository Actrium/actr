use thiserror::Error;

pub type WasmResult<T> = Result<T, WasmError>;

#[derive(Debug, Error)]
pub enum WasmError {
    #[error("WASM package verification failed: {0}")]
    VerificationFailed(#[from] crate::error::HyperError),

    #[error("WASM module load failed: {0}")]
    LoadFailed(String),

    #[error("WASM actor initialization failed: {0}")]
    InitFailed(String),

    #[error("WASM actor execution failed: {0}")]
    ExecutionFailed(String),

    #[error("WASM instance trapped (store poisoned; rebuilt on next call): {0}")]
    InstanceTrapped(String),

    #[error("WASM invocation exceeded fuel budget")]
    OutOfFuel,
    #[error("WASM invocation interrupted by epoch deadline")]
    EpochInterrupted,
    #[error("WASM resource limit exceeded: {0}")]
    ResourceLimitExceeded(&'static str),
    #[error("WASM invocation timed out after {0:?}")]
    InvocationTimeout(std::time::Duration),
}
