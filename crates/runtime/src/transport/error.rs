//! Network layer error definitions

use thiserror::Error;

/// Network layer error types
#[derive(Error, Debug)]
pub enum NetworkError {
    /// Connection error
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Signaling error
    #[error("Signaling error: {0}")]
    SignalingError(String),

    /// WebRTC error
    #[error("WebRTC error: {0}")]
    WebRtcError(String),

    /// Protocol error
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// Timeout error
    #[error("Timeout error: {0}")]
    TimeoutError(String),

    /// Authentication error
    #[error("Authentication error: {0}")]
    AuthenticationError(String),

    /// Credential expired error (requires re-registration)
    #[error("Credential expired: {0}")]
    CredentialExpired(String),

    /// Permission error
    #[error("Permission error: {0}")]
    PermissionError(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Resource exhausted error
    #[error("Resource exhausted: {0}")]
    ResourceExhaustedError(String),

    /// Network unreachable error
    #[error("Network unreachable: {0}")]
    NetworkUnreachableError(String),

    /// Service discovery error
    #[error("Service discovery error: {0}")]
    ServiceDiscoveryError(String),

    /// NAT traversal error
    #[error("NAT traversal error: {0}")]
    NatTraversalError(String),

    /// Data channel error
    #[error("Data channel error: {0}")]
    DataChannelError(String),

    /// Broadcast error
    #[error("Broadcast error: {0}")]
    BroadcastError(String),

    /// ICE error
    #[error("ICE error: {0}")]
    IceError(String),

    /// DTLS error
    #[error("DTLS error: {0}")]
    DtlsError(String),

    /// STUN/TURN error
    #[error("STUN/TURN error: {0}")]
    StunTurnError(String),

    /// WebSocket error
    #[error("WebSocket error: {0}")]
    WebSocketError(String),

    /// Connection not found error
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),

    /// Connection closed error (e.g., cancelled during creation)
    #[error("Connection closed: {0}")]
    ConnectionClosed(String),

    /// Feature not implemented error
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// Channel closed error
    #[error("Channel closed: {0}")]
    ChannelClosed(String),

    /// Send error
    #[error("Send error: {0}")]
    SendError(String),

    /// No route error
    #[error("No route: {0}")]
    NoRoute(String),

    /// Invalid operation error
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    /// Invalid argument error
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    /// Channel not found error
    #[error("Channel not found: {0}")]
    ChannelNotFound(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// URL parse error
    #[error("URL parse error: {0}")]
    UrlParseError(#[from] url::ParseError),

    /// JSON error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// Timeout error
    #[error("Timeout error: {0}")]
    Timeout(String),

    /// Other error
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

impl NetworkError {
    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            NetworkError::ConnectionError(_)
                | NetworkError::TimeoutError(_)
                | NetworkError::NetworkUnreachableError(_)
                | NetworkError::ResourceExhaustedError(_)
        )
    }

    /// Check if error is temporary
    pub fn is_temporary(&self) -> bool {
        matches!(
            self,
            NetworkError::ConnectionError(_)
                | NetworkError::TimeoutError(_)
                | NetworkError::NetworkUnreachableError(_)
                | NetworkError::ResourceExhaustedError(_)
        )
    }

    /// Check if error is fatal
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            NetworkError::AuthenticationError(_)
                | NetworkError::PermissionError(_)
                | NetworkError::ConfigurationError(_)
        )
    }

    /// Get error category
    pub fn category(&self) -> &'static str {
        match self {
            NetworkError::ConnectionError(_) => "connection",
            NetworkError::SignalingError(_) => "signaling",
            NetworkError::WebRtcError(_) => "webrtc",
            NetworkError::ProtocolError(_) => "protocol",
            NetworkError::SerializationError(_) | NetworkError::DeserializationError(_) => {
                "serialization"
            }
            NetworkError::TimeoutError(_) => "timeout",
            NetworkError::AuthenticationError(_) => "authentication",
            NetworkError::PermissionError(_) => "permission",
            NetworkError::ConfigurationError(_) => "configuration",
            NetworkError::ResourceExhaustedError(_) => "resource_exhausted",
            NetworkError::NetworkUnreachableError(_) => "network_unreachable",
            NetworkError::ServiceDiscoveryError(_) => "service_discovery",
            NetworkError::NatTraversalError(_) => "nat_traversal",
            NetworkError::DataChannelError(_) => "data_channel",
            NetworkError::IceError(_) => "ice",
            NetworkError::DtlsError(_) => "dtls",
            NetworkError::StunTurnError(_) => "stun_turn",
            NetworkError::WebSocketError(_) => "websocket",
            NetworkError::ConnectionNotFound(_) => "connection_not_found",
            NetworkError::ConnectionClosed(_) => "connection_closed",
            NetworkError::NotImplemented(_) => "not_implemented",
            NetworkError::ChannelClosed(_) => "channel_closed",
            NetworkError::SendError(_) => "send_error",
            NetworkError::NoRoute(_) => "no_route",
            NetworkError::InvalidOperation(_) => "invalid_operation",
            NetworkError::InvalidArgument(_) => "invalid_argument",
            NetworkError::ChannelNotFound(_) => "channel_not_found",
            NetworkError::IoError(_) => "io",
            NetworkError::UrlParseError(_) => "url_parse",
            NetworkError::JsonError(_) => "json",
            NetworkError::BroadcastError(_) => "broadcast",
            NetworkError::Timeout(_) => "timeout",
            NetworkError::CredentialExpired(_) => "credential_expired",
            NetworkError::Other(_) => "other",
        }
    }

    /// Get error severity (1-10, 10 is most severe)
    pub fn severity(&self) -> u8 {
        match self {
            NetworkError::ConfigurationError(_)
            | NetworkError::AuthenticationError(_)
            | NetworkError::PermissionError(_)
            | NetworkError::CredentialExpired(_) => 10,

            NetworkError::WebRtcError(_)
            | NetworkError::SignalingError(_)
            | NetworkError::ProtocolError(_) => 8,

            NetworkError::ConnectionError(_) | NetworkError::NetworkUnreachableError(_) => 7,

            NetworkError::NatTraversalError(_)
            | NetworkError::IceError(_)
            | NetworkError::DtlsError(_) => 6,

            NetworkError::TimeoutError(_) | NetworkError::ResourceExhaustedError(_) => 5,

            NetworkError::ServiceDiscoveryError(_)
            | NetworkError::DataChannelError(_)
            | NetworkError::BroadcastError(_) => 4,

            NetworkError::SerializationError(_) | NetworkError::DeserializationError(_) => 3,

            NetworkError::WebSocketError(_) | NetworkError::StunTurnError(_) => 3,

            NetworkError::ConnectionNotFound(_)
            | NetworkError::ConnectionClosed(_)
            | NetworkError::ChannelClosed(_)
            | NetworkError::SendError(_)
            | NetworkError::NoRoute(_)
            | NetworkError::ChannelNotFound(_) => 4,

            NetworkError::InvalidOperation(_) | NetworkError::InvalidArgument(_) => 6,

            NetworkError::NotImplemented(_) => 8,

            NetworkError::IoError(_)
            | NetworkError::UrlParseError(_)
            | NetworkError::JsonError(_)
            | NetworkError::Timeout(_) => 2,

            NetworkError::Other(_) => 1,
        }
    }
}

// TODO: Implement UnifiedError trait (when actr-protocol provides error_unified module)
// impl UnifiedError for NetworkError { ... }

/// Network layer result type
pub type NetworkResult<T> = Result<T, NetworkError>;

/// Convert from actor error to network error
impl From<actr_protocol::ActrError> for NetworkError {
    fn from(err: actr_protocol::ActrError) -> Self {
        NetworkError::Other(anyhow::anyhow!("Actor error: {err}"))
    }
}

/// Convert from WebRTC error
impl From<webrtc::Error> for NetworkError {
    fn from(err: webrtc::Error) -> Self {
        NetworkError::WebRtcError(err.to_string())
    }
}

/// Convert from WebSocket error
impl From<tokio_tungstenite::tungstenite::Error> for NetworkError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        NetworkError::WebSocketError(err.to_string())
    }
}

/// Convert from protobuf encode error
impl From<actr_protocol::prost::EncodeError> for NetworkError {
    fn from(err: actr_protocol::prost::EncodeError) -> Self {
        NetworkError::SerializationError(err.to_string())
    }
}

/// Convert from protobuf decode error
impl From<actr_protocol::prost::DecodeError> for NetworkError {
    fn from(err: actr_protocol::prost::DecodeError) -> Self {
        NetworkError::DeserializationError(err.to_string())
    }
}

// TODO: In future, if error statistics needed, can add ErrorStats struct
// Recommend using arrays instead of HashMap (error categories and severities are fixed)
