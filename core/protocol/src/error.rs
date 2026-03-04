#![allow(deprecated)]
//! Error types for Actor-RTC protocol

use crate::actr_ext::ActrError;
use crate::name::NameError;

/// Protocol-level errors primarily related to data structure and format validity.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("Actor identity error: {0}")]
    Actr(#[from] ActrError),

    #[error("URI parsing error: {0}")]
    Uri(#[from] crate::uri::ActrUriError),

    #[error("Invalid name: {0}")]
    Name(#[from] NameError),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Decode error: {0}")]
    DecodeError(String),

    #[error("Encode error: {0}")]
    EncodeError(String),

    #[error("Unknown route: {0}")]
    UnknownRoute(String),

    #[error("Transport error: {0}")]
    TransportError(String),

    #[error("Timeout")]
    Timeout,

    #[error("Target not found: {0}")]
    TargetNotFound(String),

    #[error("Target unavailable: {0}")]
    TargetUnavailable(String),

    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),
}

/// Convenient result type for protocol operations
pub type ProtocolResult<T> = Result<T, ProtocolError>;

/// Actor result type - commonly used in framework and runtime
pub type ActorResult<T> = Result<T, ProtocolError>;
