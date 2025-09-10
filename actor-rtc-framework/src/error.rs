//! 错误类型定义

use thiserror::Error;

/// Actor 系统错误类型
#[derive(Error, Debug)]
pub enum ActorError {
    #[error("Actor system not initialized")]
    NotInitialized,

    #[error("Actor already attached")]
    AlreadyAttached,

    #[error("Signaling connection failed: {0}")]
    SignalingFailed(String),

    #[error("WebRTC connection failed: {0}")]
    WebRTCFailed(String),

    #[error("Message routing failed: {0}")]
    RoutingFailed(String),

    #[error("Actor not found: {actor_id}")]
    ActorNotFound { actor_id: String },

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Invalid message format: {0}")]
    InvalidMessage(String),

    #[error("Context operation failed: {0}")]
    ContextError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error("System shutdown")]
    SystemShutdown,

    #[error("Business logic error: {0}")]
    Business(String),
}

/// 信令错误类型
#[derive(Error, Debug)]
pub enum SignalingError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Message parse error: {0}")]
    ParseError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),
}

/// WebRTC 错误类型  
#[derive(Error, Debug)]
pub enum WebRTCError {
    #[error("Peer connection failed: {0}")]
    PeerConnectionFailed(String),

    #[error("Data channel error: {0}")]
    DataChannelError(String),

    #[error("ICE gathering failed: {0}")]
    IceGatheringFailed(String),

    #[error("SDP negotiation failed: {0}")]
    SdpNegotiationFailed(String),

    #[error("Media error: {0}")]
    MediaError(String),
}

/// 消息处理错误
#[derive(Error, Debug)]
pub enum MessageError {
    #[error("Message encoding failed: {0}")]
    EncodingFailed(String),

    #[error("Message decoding failed: {0}")]
    DecodingFailed(String),

    #[error("Message dispatch failed: {0}")]
    DispatchFailed(String),

    #[error("Handler not found for message type: {0}")]
    HandlerNotFound(String),

    #[error("Message too large: {size} bytes")]
    MessageTooLarge { size: usize },
}

/// 框架结果类型
pub type ActorResult<T> = Result<T, ActorError>;

impl From<SignalingError> for ActorError {
    fn from(err: SignalingError) -> Self {
        ActorError::SignalingFailed(err.to_string())
    }
}

impl From<WebRTCError> for ActorError {
    fn from(err: WebRTCError) -> Self {
        ActorError::WebRTCFailed(err.to_string())
    }
}

impl From<MessageError> for ActorError {
    fn from(err: MessageError) -> Self {
        ActorError::RoutingFailed(err.to_string())
    }
}
