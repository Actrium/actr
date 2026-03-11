use thiserror::Error;

pub type DynclibResult<T> = Result<T, DynclibError>;

#[derive(Debug, Error)]
pub enum DynclibError {
    #[error("failed to load library: {0}")]
    LoadFailed(String),

    #[error("missing symbol '{symbol}' in library: {detail}")]
    MissingSymbol { symbol: String, detail: String },

    #[error("init failed with code {0}")]
    InitFailed(i32),

    #[error("dispatch failed: {0}")]
    DispatchFailed(String),

    #[error("protocol error: {0}")]
    ProtocolError(String),
}
