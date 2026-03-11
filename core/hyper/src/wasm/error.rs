use thiserror::Error;

pub type WasmResult<T> = Result<T, WasmError>;

#[derive(Debug, Error)]
pub enum WasmError {
    #[error("WASM 模块加载失败: {0}")]
    LoadFailed(String),

    #[error("WASM actor 初始化失败: {0}")]
    InitFailed(String),

    #[error("WASM actor 执行失败: {0}")]
    ExecutionFailed(String),
}
