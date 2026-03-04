//! Error types for actr

/// Error type for actr operations
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ActrError {
    #[error("Configuration error: {msg}")]
    ConfigError { msg: String },

    #[error("Connection error: {msg}")]
    ConnectionError { msg: String },

    #[error("RPC error: {msg}")]
    RpcError { msg: String },

    #[error("State error: {msg}")]
    StateError { msg: String },

    #[error("Internal error: {msg}")]
    InternalError { msg: String },

    #[error("Timeout error: {msg}")]
    TimeoutError { msg: String },

    #[error("Workload error: {msg}")]
    WorkloadError { msg: String },
}

pub type ActrResult<T> = Result<T, ActrError>;

impl From<actr_protocol::ProtocolError> for ActrError {
    fn from(e: actr_protocol::ProtocolError) -> Self {
        ActrError::RpcError { msg: e.to_string() }
    }
}

impl From<ActrError> for actr_protocol::ProtocolError {
    fn from(e: ActrError) -> Self {
        match e {
            ActrError::ConfigError { msg } => {
                actr_protocol::ProtocolError::InvalidStateTransition(msg)
            }
            ActrError::ConnectionError { msg } => actr_protocol::ProtocolError::TransportError(msg),
            ActrError::RpcError { msg } => actr_protocol::ProtocolError::TransportError(msg),
            ActrError::StateError { msg } => {
                actr_protocol::ProtocolError::InvalidStateTransition(msg)
            }
            ActrError::InternalError { msg } => {
                actr_protocol::ProtocolError::InvalidStateTransition(msg)
            }
            ActrError::TimeoutError { .. } => actr_protocol::ProtocolError::Timeout,
            ActrError::WorkloadError { msg } => {
                actr_protocol::ProtocolError::InvalidStateTransition(msg)
            }
        }
    }
}
