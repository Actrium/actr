use thiserror::Error;

pub type WasmResult<T> = Result<T, WasmError>;

#[derive(Debug, Error)]
pub enum WasmError {
    #[error("WASM module load failed: {0}")]
    LoadFailed(String),

    #[error("WASM actor initialization failed: {0}")]
    InitFailed(String),

    #[error("WASM actor execution failed: {0}")]
    ExecutionFailed(String),
}
