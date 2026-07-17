//! Network layer error definitions

use actr_protocol::{ActrError, Classify, ErrorKind};
use std::time::Duration;
use thiserror::Error;

/// Typed authentication verdict extracted from a transport failure.
///
/// This is the correctness-critical distinction the membership authority relies
/// on: a verdict means the *credential itself* was rejected (or the realm is
/// gone), so retrying the SAME credential is pointless — the recovery engine
/// must mint a fresh one. Transport blips (timeouts, closed sockets, transient
/// connection errors) never produce a verdict; they stay on the plain backoff
/// path and retry the existing credential.
///
/// Kept deliberately small: only the outcomes that a signaling handshake or
/// heartbeat can surface as an *authentication* decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthVerdict {
    /// The credential was presented and rejected (handshake / heartbeat 401).
    ///
    /// Refresh-eligible: the recovery engine should re-acquire (soft renew,
    /// then hard `/register` fallback) against the stable node identity.
    Rejected,
    /// The realm returned 403 — membership in this realm is denied.
    ///
    /// Terminal for automatic recovery: no amount of re-acquiring a credential
    /// helps because the realm itself refuses this node. The controller enters
    /// a loud, slow-cadence `Denied` phase rather than a tight retry loop.
    RealmDenied,
}

/// Reason a credential was rejected, for diagnostics / metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    /// The signaling WebSocket handshake returned HTTP 401.
    Handshake401,
}

impl std::fmt::Display for RejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectionReason::Handshake401 => f.write_str("handshake_401"),
        }
    }
}

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

    /// The credential was presented and rejected (handshake / heartbeat 401).
    ///
    /// Distinct from `CredentialExpired`/`AuthenticationError`: this variant is
    /// the typed hook the membership authority branches on via
    /// [`NetworkError::auth_verdict`]. Carries the reason so metrics can tell a
    /// handshake 401 from a heartbeat 401.
    #[error("Credential rejected ({reason}): {message}")]
    CredentialRejected {
        reason: RejectionReason,
        message: String,
    },

    /// The realm returned HTTP 403 — membership denied. Terminal for automatic
    /// recovery (see [`AuthVerdict::RealmDenied`]).
    #[error("Realm denied: {0}")]
    RealmDenied(String),

    /// The signaling server is up but not ready to authenticate yet (HTTP 503).
    ///
    /// Explicitly NOT an auth verdict: the credential is fine, the server just
    /// asked us to come back later. Carries an optional `Retry-After` so the
    /// reconnect loop can honor the server's backoff hint and retry the SAME
    /// credential.
    #[error("Server not ready: {message}")]
    ServerNotReady {
        message: String,
        retry_after: Option<Duration>,
    },

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

    /// Data channel closed error
    #[error("Data channel closed: {0}")]
    DataChannelClosed(String),

    /// Data channel exists but is not currently open/sendable
    #[error("Data channel not open: {0}")]
    DataChannelNotOpen(String),

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

    /// WebSocket closed error
    #[error("WebSocket closed: {0}")]
    WebSocketClosed(String),

    /// Connection not found error
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),

    /// Connection closed error (e.g., cancelled during creation)
    #[error("Connection closed: {0}")]
    ConnectionClosed(String),

    /// WebRTC peer connection closed error
    #[error("Peer connection closed: {0}")]
    PeerConnectionClosed(String),

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

    /// Other error
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

impl Classify for NetworkError {
    fn kind(&self) -> ErrorKind {
        match self {
            // Transient: connection-level failures that may resolve on retry
            NetworkError::ConnectionError(_)
            | NetworkError::ConnectionClosed(_)
            | NetworkError::PeerConnectionClosed(_)
            | NetworkError::ChannelClosed(_)
            | NetworkError::DataChannelClosed(_)
            | NetworkError::DataChannelNotOpen(_)
            | NetworkError::SendError(_)
            | NetworkError::NetworkUnreachableError(_)
            | NetworkError::ResourceExhaustedError(_)
            | NetworkError::WebSocketError(_)
            | NetworkError::WebSocketClosed(_)
            | NetworkError::SignalingError(_)
            | NetworkError::WebRtcError(_)
            | NetworkError::NatTraversalError(_)
            | NetworkError::IceError(_) => ErrorKind::Transient,

            // Transient: timeout (framework-internal; caller-set deadlines should be Client)
            NetworkError::TimeoutError(_) => ErrorKind::Transient,

            // Transient: server up but not ready to authenticate; retry same credential.
            NetworkError::ServerNotReady { .. } => ErrorKind::Transient,

            // Client: caller or config errors that won't fix themselves
            NetworkError::ConnectionNotFound(_)
            | NetworkError::ChannelNotFound(_)
            | NetworkError::NoRoute(_)
            | NetworkError::InvalidArgument(_)
            | NetworkError::InvalidOperation(_)
            | NetworkError::ConfigurationError(_)
            | NetworkError::ServiceDiscoveryError(_) => ErrorKind::Client,

            // Client: auth/permission. `CredentialRejected` / `RealmDenied` are
            // Client-kind (retrying the SAME credential never succeeds) but the
            // `From<NetworkError> for ActrError` boundary gives them a dedicated
            // arm so they surface as `PermissionDenied`, not `NotFound`.
            NetworkError::AuthenticationError(_)
            | NetworkError::PermissionError(_)
            | NetworkError::CredentialExpired(_)
            | NetworkError::CredentialRejected { .. }
            | NetworkError::RealmDenied(_) => ErrorKind::Client,

            // Corrupt: data cannot be decoded
            NetworkError::DeserializationError(_) => ErrorKind::Corrupt,

            // Internal: framework-level issues
            NetworkError::ProtocolError(_)
            | NetworkError::SerializationError(_)
            | NetworkError::DataChannelError(_)
            | NetworkError::BroadcastError(_)
            | NetworkError::DtlsError(_)
            | NetworkError::StunTurnError(_)
            | NetworkError::NotImplemented(_)
            | NetworkError::IoError(_)
            | NetworkError::UrlParseError(_)
            | NetworkError::JsonError(_)
            | NetworkError::Other(_) => ErrorKind::Internal,
        }
    }
}

impl NetworkError {
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
            NetworkError::DataChannelClosed(_) => "data_channel_closed",
            NetworkError::DataChannelNotOpen(_) => "data_channel_not_open",
            NetworkError::IceError(_) => "ice",
            NetworkError::DtlsError(_) => "dtls",
            NetworkError::StunTurnError(_) => "stun_turn",
            NetworkError::WebSocketError(_) => "websocket",
            NetworkError::WebSocketClosed(_) => "websocket_closed",
            NetworkError::ConnectionNotFound(_) => "connection_not_found",
            NetworkError::ConnectionClosed(_) => "connection_closed",
            NetworkError::PeerConnectionClosed(_) => "peer_connection_closed",
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
            NetworkError::CredentialExpired(_) => "credential_expired",
            NetworkError::CredentialRejected { .. } => "credential_rejected",
            NetworkError::RealmDenied(_) => "realm_denied",
            NetworkError::ServerNotReady { .. } => "server_not_ready",
            NetworkError::Other(_) => "other",
        }
    }

    /// Get error severity (1-10, 10 is most severe)
    pub fn severity(&self) -> u8 {
        match self {
            NetworkError::ConfigurationError(_)
            | NetworkError::AuthenticationError(_)
            | NetworkError::PermissionError(_)
            | NetworkError::CredentialExpired(_)
            | NetworkError::CredentialRejected { .. }
            | NetworkError::RealmDenied(_) => 10,

            NetworkError::ServerNotReady { .. } => 5,

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
            | NetworkError::PeerConnectionClosed(_)
            | NetworkError::ChannelClosed(_)
            | NetworkError::DataChannelClosed(_)
            | NetworkError::DataChannelNotOpen(_)
            | NetworkError::WebSocketClosed(_)
            | NetworkError::SendError(_)
            | NetworkError::NoRoute(_)
            | NetworkError::ChannelNotFound(_) => 4,

            NetworkError::InvalidOperation(_) | NetworkError::InvalidArgument(_) => 6,

            NetworkError::NotImplemented(_) => 8,

            NetworkError::IoError(_)
            | NetworkError::UrlParseError(_)
            | NetworkError::JsonError(_) => 2,

            NetworkError::Other(_) => 1,
        }
    }

    /// Return true when the error means the underlying transport is no
    /// longer sendable and should be treated as a stale/closed candidate.
    ///
    /// `ChannelClosed` is intentionally excluded: it is a generic in-process
    /// channel failure and existing non-RPC send paths surface it directly
    /// instead of treating it as a stale WebRTC lane to self-heal.
    ///
    /// Exhaustive by design (mirrors `kind`/`category`/`severity`): adding a
    /// new `NetworkError` variant forces the author to decide its closed-like
    /// status here, so a future closed-transport variant cannot silently miss
    /// stale-candidate self-heal.
    pub fn is_closed_like(&self) -> bool {
        match self {
            // Transport is gone / not sendable: self-heal by evicting the stale candidate.
            NetworkError::ConnectionClosed(_)
            | NetworkError::PeerConnectionClosed(_)
            | NetworkError::DataChannelClosed(_)
            | NetworkError::DataChannelNotOpen(_)
            | NetworkError::WebSocketClosed(_) => true,

            // Not a stale-transport signal.
            NetworkError::ConnectionError(_)
            | NetworkError::SignalingError(_)
            | NetworkError::WebRtcError(_)
            | NetworkError::ProtocolError(_)
            | NetworkError::SerializationError(_)
            | NetworkError::DeserializationError(_)
            | NetworkError::TimeoutError(_)
            | NetworkError::AuthenticationError(_)
            | NetworkError::CredentialExpired(_)
            | NetworkError::CredentialRejected { .. }
            | NetworkError::RealmDenied(_)
            | NetworkError::ServerNotReady { .. }
            | NetworkError::PermissionError(_)
            | NetworkError::ConfigurationError(_)
            | NetworkError::ResourceExhaustedError(_)
            | NetworkError::NetworkUnreachableError(_)
            | NetworkError::ServiceDiscoveryError(_)
            | NetworkError::NatTraversalError(_)
            | NetworkError::DataChannelError(_)
            | NetworkError::BroadcastError(_)
            | NetworkError::IceError(_)
            | NetworkError::DtlsError(_)
            | NetworkError::StunTurnError(_)
            | NetworkError::WebSocketError(_)
            | NetworkError::ConnectionNotFound(_)
            | NetworkError::NotImplemented(_)
            | NetworkError::ChannelClosed(_)
            | NetworkError::SendError(_)
            | NetworkError::NoRoute(_)
            | NetworkError::InvalidOperation(_)
            | NetworkError::InvalidArgument(_)
            | NetworkError::ChannelNotFound(_)
            | NetworkError::IoError(_)
            | NetworkError::UrlParseError(_)
            | NetworkError::JsonError(_)
            | NetworkError::Other(_) => false,
        }
    }

    /// Classify this error as a typed authentication verdict, if any.
    ///
    /// This is the single branch point the membership authority relies on:
    ///
    /// - `Some(AuthVerdict::Rejected)` — the credential was presented and
    ///   rejected (handshake / heartbeat 401). Re-acquire is warranted.
    /// - `Some(AuthVerdict::RealmDenied)` — realm 403. Terminal for automatic
    ///   recovery.
    /// - `None` — NOT an auth decision. Includes `ServerNotReady` (503; the
    ///   credential is fine, the server asked us to retry later) and every
    ///   transport-level failure. These stay on the plain backoff / retry-same-
    ///   credential path and MUST never reach the credential owner.
    ///
    /// Exhaustive by design so a future auth-shaped variant forces an explicit
    /// verdict decision here.
    pub fn auth_verdict(&self) -> Option<AuthVerdict> {
        match self {
            NetworkError::CredentialRejected { .. } => Some(AuthVerdict::Rejected),
            NetworkError::RealmDenied(_) => Some(AuthVerdict::RealmDenied),

            // Not an auth verdict — must not trigger re-acquire.
            NetworkError::ServerNotReady { .. }
            | NetworkError::ConnectionError(_)
            | NetworkError::SignalingError(_)
            | NetworkError::WebRtcError(_)
            | NetworkError::ProtocolError(_)
            | NetworkError::SerializationError(_)
            | NetworkError::DeserializationError(_)
            | NetworkError::TimeoutError(_)
            | NetworkError::AuthenticationError(_)
            | NetworkError::CredentialExpired(_)
            | NetworkError::PermissionError(_)
            | NetworkError::ConfigurationError(_)
            | NetworkError::ResourceExhaustedError(_)
            | NetworkError::NetworkUnreachableError(_)
            | NetworkError::ServiceDiscoveryError(_)
            | NetworkError::NatTraversalError(_)
            | NetworkError::DataChannelError(_)
            | NetworkError::DataChannelClosed(_)
            | NetworkError::DataChannelNotOpen(_)
            | NetworkError::BroadcastError(_)
            | NetworkError::IceError(_)
            | NetworkError::DtlsError(_)
            | NetworkError::StunTurnError(_)
            | NetworkError::WebSocketError(_)
            | NetworkError::WebSocketClosed(_)
            | NetworkError::ConnectionNotFound(_)
            | NetworkError::ConnectionClosed(_)
            | NetworkError::PeerConnectionClosed(_)
            | NetworkError::NotImplemented(_)
            | NetworkError::ChannelClosed(_)
            | NetworkError::SendError(_)
            | NetworkError::NoRoute(_)
            | NetworkError::InvalidOperation(_)
            | NetworkError::InvalidArgument(_)
            | NetworkError::ChannelNotFound(_)
            | NetworkError::IoError(_)
            | NetworkError::UrlParseError(_)
            | NetworkError::JsonError(_)
            | NetworkError::Other(_) => None,
        }
    }

    /// The server-provided `Retry-After` hint, when the error carries one.
    ///
    /// Only `ServerNotReady` (503) currently carries a hint; all other variants
    /// return `None`.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            NetworkError::ServerNotReady { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

// TODO: Implement UnifiedError trait (when actr-protocol provides error_unified module)
// impl UnifiedError for NetworkError { ... }

/// Network layer result type
pub type NetworkResult<T> = Result<T, NetworkError>;

/// Convert from `ActrIdError` (identity parsing) to `NetworkError`
impl From<actr_protocol::ActrIdError> for NetworkError {
    fn from(err: actr_protocol::ActrIdError) -> Self {
        NetworkError::InvalidArgument(err.to_string())
    }
}

/// Convert `NetworkError` to the public top-level `ActrError`.
///
/// This is the single boundary where transport failures become user-visible errors.
impl From<NetworkError> for ActrError {
    fn from(err: NetworkError) -> Self {
        // Preserve specific variants where the protocol surface has a precise
        // counterpart (e.g. caller-deadline TimedOut), so binding consumers can
        // branch on the exact failure mode instead of a coarse Unavailable.
        match &err {
            NetworkError::TimeoutError(_) => return ActrError::TimedOut,
            NetworkError::PermissionError(msg)
            | NetworkError::AuthenticationError(msg)
            | NetworkError::CredentialExpired(msg)
            | NetworkError::RealmDenied(msg) => {
                return ActrError::PermissionDenied(msg.clone());
            }
            // Dedicated arm so the Client-kind auth-rejection does not collapse
            // into `NotFound` (which monitoring/consumers read as "route gone").
            NetworkError::CredentialRejected { message, .. } => {
                return ActrError::PermissionDenied(message.clone());
            }
            // Server asked us to come back later — surface as a transient
            // Unavailable so RPC callers retry rather than treat it as fatal.
            NetworkError::ServerNotReady { message, .. } => {
                return ActrError::Unavailable(message.clone());
            }
            NetworkError::NoRoute(msg)
            | NetworkError::ConnectionNotFound(msg)
            | NetworkError::ChannelNotFound(msg)
            | NetworkError::ServiceDiscoveryError(msg) => {
                return ActrError::NotFound(msg.clone());
            }
            NetworkError::InvalidArgument(msg) | NetworkError::InvalidOperation(msg) => {
                return ActrError::InvalidArgument(msg.clone());
            }
            NetworkError::ConfigurationError(msg) => {
                return ActrError::Internal(msg.clone());
            }
            _ => {}
        }
        match err.kind() {
            ErrorKind::Transient => ActrError::Unavailable(err.to_string()),
            ErrorKind::Client => ActrError::NotFound(err.to_string()),
            ErrorKind::Corrupt => ActrError::DecodeFailure(err.to_string()),
            ErrorKind::Internal => ActrError::Internal(err.to_string()),
        }
    }
}

/// Convert from WebRTC error
///
/// Closed / not-open variants are mapped structurally so any future `?` on
/// a webrtc call in a send path cannot regress to an unstructured
/// `WebRtcError` that `is_closed_like()` would miss. The closed set is kept
/// minimal (connection-level errors only); channel-level closed errors are
/// classified with state context by `classify_data_channel_send_error`.
impl From<webrtc::Error> for NetworkError {
    fn from(err: webrtc::Error) -> Self {
        match &err {
            webrtc::Error::ErrConnectionClosed | webrtc::Error::ErrClosedPipe => {
                NetworkError::PeerConnectionClosed(err.to_string())
            }
            webrtc::Error::ErrDataChannelNotOpen | webrtc::Error::ErrSCTPNotEstablished => {
                NetworkError::DataChannelNotOpen(err.to_string())
            }
            _ => NetworkError::WebRtcError(err.to_string()),
        }
    }
}

/// Whether a tungstenite WebSocket error indicates the connection is closed.
///
/// Shared by `From<WsError>` and the WebSocket lane send path so the
/// closed-variant set (`ConnectionClosed` / `AlreadyClosed`) is declared once.
pub(crate) fn is_tungstenite_closed(err: &tokio_tungstenite::tungstenite::Error) -> bool {
    matches!(
        err,
        tokio_tungstenite::tungstenite::Error::ConnectionClosed
            | tokio_tungstenite::tungstenite::Error::AlreadyClosed
    )
}

/// Parse an HTTP `Retry-After` header value that carries a delta-seconds count.
///
/// Only the numeric (delta-seconds) form is honored; an HTTP-date form returns
/// `None` (the caller falls back to its own backoff). Kept narrow on purpose.
fn parse_retry_after_seconds(headers: &tokio_tungstenite::tungstenite::http::HeaderMap) -> Option<Duration> {
    let value = headers.get(tokio_tungstenite::tungstenite::http::header::RETRY_AFTER)?;
    let secs: u64 = value.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

/// Convert from WebSocket error.
///
/// The critical case is `Error::Http(resp)`: the signaling handshake was
/// answered with an HTTP status instead of a 101 upgrade. Previously every such
/// failure was flattened to `WebSocketError` (a transient), so a 401 credential
/// rejection was indistinguishable from a network blip and the reconnect loop
/// retried the SAME dead credential forever. We now inspect the status and emit
/// a typed variant the membership authority can branch on via `auth_verdict()`.
impl From<tokio_tungstenite::tungstenite::Error> for NetworkError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        use tokio_tungstenite::tungstenite::Error as WsError;
        use tokio_tungstenite::tungstenite::http::StatusCode;

        if is_tungstenite_closed(&err) {
            return NetworkError::WebSocketClosed(err.to_string());
        }

        if let WsError::Http(resp) = &err {
            let status = resp.status();
            let body_hint = resp
                .body()
                .as_ref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| format!(": {s}"))
                .unwrap_or_default();

            return match status {
                StatusCode::UNAUTHORIZED => NetworkError::CredentialRejected {
                    reason: RejectionReason::Handshake401,
                    message: format!("signaling handshake returned 401{body_hint}"),
                },
                StatusCode::FORBIDDEN => {
                    NetworkError::RealmDenied(format!("signaling handshake returned 403{body_hint}"))
                }
                StatusCode::SERVICE_UNAVAILABLE => NetworkError::ServerNotReady {
                    message: format!("signaling handshake returned 503{body_hint}"),
                    retry_after: parse_retry_after_seconds(resp.headers()),
                },
                other => NetworkError::WebSocketError(format!(
                    "signaling handshake returned HTTP {other}{body_hint}"
                )),
            };
        }

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

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
