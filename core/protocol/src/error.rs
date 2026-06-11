//! Top-level error types for the Actor-RTC framework.
//!
//! ## Design
//!
//! Two layers only:
//!
//! ```text
//! NetworkError   (transport-internal, never exposed to users)
//!      ↓  From
//! ActrError      (public, flat enum — what callers see)
//! ```
//!
//! `RuntimeError` and `ProtocolError` have been removed.
//!
//! ## Error classification
//!
//! Every error belongs to one fault domain (`ErrorKind`):
//!
//! | Kind      | Meaning                        | Retry? | DLQ? |
//! |-----------|--------------------------------|--------|------|
//! | Transient | Environmental fluctuation      | yes    | no   |
//! | Client    | Caller error (bad request)     | no     | no   |
//! | Internal  | Framework bug / panic          | no     | no   |
//! | Corrupt   | Data corruption                | no     | yes  |
//!
//! Use the `Classify` trait to query classification from any error type.

use crate::ActrId;
use std::fmt;
use thiserror::Error;

// ── RecoveryInfo ───────────────────────────────────────────────────────────────

/// Stable machine-readable code for a connection recovery window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryCode {
    PeerDisconnected,
    PeerFailed,
    IceNetworkStarted,
    RecoveryTimeout,
    TransportClosing,
}

impl fmt::Display for RecoveryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoveryCode::PeerDisconnected => write!(f, "PeerDisconnected"),
            RecoveryCode::PeerFailed => write!(f, "PeerFailed"),
            RecoveryCode::IceNetworkStarted => write!(f, "IceNetworkStarted"),
            RecoveryCode::RecoveryTimeout => write!(f, "RecoveryTimeout"),
            RecoveryCode::TransportClosing => write!(f, "TransportClosing"),
        }
    }
}

/// Whether the failed operation may have reached the remote peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    /// The operation failed before being written to the transport.
    NotSent,
    /// The transport state changed after send started, so delivery is unknown.
    DeliveryUncertain,
}

impl fmt::Display for DeliveryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeliveryState::NotSent => write!(f, "NotSent"),
            DeliveryState::DeliveryUncertain => write!(f, "DeliveryUncertain"),
        }
    }
}

/// Structured information for a connection recovery window.
///
/// `delivery` is intentionally part of the public payload. For preflight
/// recovery errors it is `NotSent`, which tells callers that retrying creates a
/// fresh operation rather than duplicating one already delivered remotely.
#[derive(Debug, Clone)]
pub struct RecoveryInfo {
    pub peer: ActrId,
    pub session_id: Option<u64>,
    pub code: RecoveryCode,
    pub reason: String,
    pub elapsed_ms: u64,
    pub timeout_ms: u64,
    pub retry_after_ms: Option<u64>,
    pub delivery: DeliveryState,
}

impl RecoveryInfo {
    pub fn new(
        peer: ActrId,
        session_id: Option<u64>,
        code: RecoveryCode,
        reason: impl Into<String>,
        elapsed_ms: u64,
        timeout_ms: u64,
    ) -> Self {
        let retry_after_ms = timeout_ms.checked_sub(elapsed_ms);
        Self {
            peer,
            session_id,
            code,
            reason: reason.into(),
            elapsed_ms,
            timeout_ms,
            retry_after_ms,
            delivery: DeliveryState::NotSent,
        }
    }
}

impl fmt::Display for RecoveryInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = if self.code == RecoveryCode::RecoveryTimeout {
            "Connection recovery timeout"
        } else {
            "Connection recovering"
        };
        write!(
            f,
            "{prefix}: peer={:?}, session_id={:?}, code={}, reason={}, elapsed_ms={}, timeout_ms={}, retry_after_ms={:?}, delivery={}",
            self.peer,
            self.session_id,
            self.code,
            self.reason,
            self.elapsed_ms,
            self.timeout_ms,
            self.retry_after_ms,
            self.delivery
        )
    }
}

// ── ActrError ────────────────────────────────────────────────────────────────

/// Top-level framework error, returned to all callers.
///
/// Flat enum — no nested error wrapping. Each variant is self-describing.
#[derive(Error, Debug, Clone)]
pub enum ActrError {
    // ── Transient ──────────────────────────────────────────────────────────
    /// Peer temporarily unavailable: connection lost, overloaded, or reconnecting.
    ///
    /// `ErrorKind::Transient` — retry with backoff.
    #[error("unavailable: {0}")]
    Unavailable(String),

    /// Connection is in a recovery window.
    ///
    /// `ErrorKind::Transient` — retry within the recovery window; the peer
    /// should become reachable again once ICE restart completes.
    #[error("recovering: {0}")]
    Recovering(RecoveryInfo),

    /// Request deadline exceeded.
    ///
    /// `ErrorKind::Transient` — may retry with a fresh deadline.
    #[error("timed out")]
    TimedOut,

    // ── Client ─────────────────────────────────────────────────────────────
    /// Target actor not found.
    ///
    /// `ErrorKind::Client` — do not retry; check service discovery first.
    #[error("not found: {0}")]
    NotFound(String),

    /// Permission denied by ACL.
    ///
    /// `ErrorKind::Client` — do not retry; fix authorization.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Invalid argument or malformed request.
    ///
    /// `ErrorKind::Client` — do not retry; fix the request.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// No handler registered for the given route key.
    ///
    /// `ErrorKind::Client` — do not retry; check service definition.
    #[error("unknown route: {0}")]
    UnknownRoute(String),

    /// Required dependency not found in the lock file.
    ///
    /// `ErrorKind::Client` — do not retry; fix the manifest.
    #[error("dependency '{service_name}' not found: {message}")]
    DependencyNotFound {
        service_name: String,
        message: String,
    },

    // ── Corrupt ────────────────────────────────────────────────────────────
    /// Protobuf decode failure — message data is corrupted.
    ///
    /// `ErrorKind::Corrupt` — route to Dead Letter Queue; do not retry.
    #[error("decode failure: {0}")]
    DecodeFailure(String),

    // ── Internal ───────────────────────────────────────────────────────────
    /// Feature not yet implemented.
    ///
    /// `ErrorKind::Internal` — do not retry.
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// Internal framework error: bug, panic, or unrecoverable state.
    ///
    /// `ErrorKind::Internal` — do not retry; investigate logs.
    #[error("internal error: {0}")]
    Internal(String),
}

// ── ErrorKind ────────────────────────────────────────────────────────────────

/// Fault domain classification for any framework error.
///
/// All error types implement [`Classify`] to expose their `ErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Environmental fluctuation — retry with exponential backoff.
    Transient,
    /// Caller error — bad request or system state; do not retry.
    Client,
    /// Framework bug or panic — do not retry; alert.
    Internal,
    /// Data corruption — route to Dead Letter Queue; manual intervention required.
    Corrupt,
}

// ── Classify trait ───────────────────────────────────────────────────────────

/// Fault-domain classification for error types.
///
/// Implement `kind()` only; `is_retryable()` and `requires_dlq()` have
/// correct default implementations derived from `kind()`.
pub trait Classify {
    /// Returns the fault domain this error belongs to.
    fn kind(&self) -> ErrorKind;

    /// Returns `true` if the operation may be retried.
    ///
    /// Only `ErrorKind::Transient` errors are retryable.
    fn is_retryable(&self) -> bool {
        matches!(self.kind(), ErrorKind::Transient)
    }

    /// Returns `true` if the message should be routed to the Dead Letter Queue.
    ///
    /// Only `ErrorKind::Corrupt` errors require DLQ routing.
    fn requires_dlq(&self) -> bool {
        matches!(self.kind(), ErrorKind::Corrupt)
    }
}

impl Classify for ActrError {
    fn kind(&self) -> ErrorKind {
        match self {
            ActrError::Unavailable(_) | ActrError::Recovering(_) | ActrError::TimedOut => {
                ErrorKind::Transient
            }

            ActrError::NotFound(_)
            | ActrError::PermissionDenied(_)
            | ActrError::InvalidArgument(_)
            | ActrError::UnknownRoute(_)
            | ActrError::DependencyNotFound { .. } => ErrorKind::Client,

            ActrError::DecodeFailure(_) => ErrorKind::Corrupt,

            ActrError::NotImplemented(_) | ActrError::Internal(_) => ErrorKind::Internal,
        }
    }
}

// ── Convenience type aliases ──────────────────────────────────────────────────

/// Result type for actor RPC calls.
pub type ActorResult<T> = Result<T, ActrError>;

#[cfg(test)]
mod tests {
    use super::*;

    // ── ActrError::kind() classification ─────────────────────────────────────

    #[test]
    fn transient_variants_classify_correctly() {
        assert_eq!(
            ActrError::Unavailable("x".into()).kind(),
            ErrorKind::Transient
        );
        assert_eq!(ActrError::TimedOut.kind(), ErrorKind::Transient);
    }

    #[test]
    fn client_variants_classify_correctly() {
        assert_eq!(ActrError::NotFound("x".into()).kind(), ErrorKind::Client);
        assert_eq!(
            ActrError::PermissionDenied("x".into()).kind(),
            ErrorKind::Client
        );
        assert_eq!(
            ActrError::InvalidArgument("x".into()).kind(),
            ErrorKind::Client
        );
        assert_eq!(
            ActrError::UnknownRoute("x".into()).kind(),
            ErrorKind::Client
        );
        assert_eq!(
            ActrError::DependencyNotFound {
                service_name: "svc".into(),
                message: "not found".into(),
            }
            .kind(),
            ErrorKind::Client
        );
    }

    #[test]
    fn corrupt_variant_classifies_correctly() {
        assert_eq!(
            ActrError::DecodeFailure("x".into()).kind(),
            ErrorKind::Corrupt
        );
    }

    #[test]
    fn internal_variants_classify_correctly() {
        assert_eq!(
            ActrError::NotImplemented("x".into()).kind(),
            ErrorKind::Internal
        );
        assert_eq!(ActrError::Internal("x".into()).kind(), ErrorKind::Internal);
    }

    // ── Classify default impls ────────────────────────────────────────────────

    #[test]
    fn only_transient_is_retryable() {
        assert!(ActrError::Unavailable("x".into()).is_retryable());
        assert!(ActrError::TimedOut.is_retryable());

        assert!(!ActrError::NotFound("x".into()).is_retryable());
        assert!(!ActrError::DecodeFailure("x".into()).is_retryable());
        assert!(!ActrError::Internal("x".into()).is_retryable());
    }

    #[test]
    fn only_corrupt_requires_dlq() {
        assert!(ActrError::DecodeFailure("x".into()).requires_dlq());

        assert!(!ActrError::Unavailable("x".into()).requires_dlq());
        assert!(!ActrError::TimedOut.requires_dlq());
        assert!(!ActrError::NotFound("x".into()).requires_dlq());
        assert!(!ActrError::Internal("x".into()).requires_dlq());
    }

    // ── Clone ─────────────────────────────────────────────────────────────────

    #[test]
    fn actr_error_is_clone() {
        let e = ActrError::InvalidArgument("bad".into());
        let cloned = e.clone();
        assert_eq!(format!("{cloned}"), "invalid argument: bad");
    }

    // ── RecoveryInfo Display ──────────────────────────────────────────────

    #[test]
    fn recovery_info_peer_disconnected_display() {
        let peer = ActrId::default();
        let info = RecoveryInfo::new(
            peer.clone(),
            Some(42),
            RecoveryCode::PeerDisconnected,
            "peer state Disconnected",
            1200,
            6000,
        );
        let s = format!("{info}");
        assert!(s.starts_with("Connection recovering: peer="));
        assert!(s.contains("session_id=Some(42)"));
        assert!(s.contains("code=PeerDisconnected"));
        assert!(s.contains("reason=peer state Disconnected"));
        assert!(s.contains("elapsed_ms=1200"));
        assert!(s.contains("timeout_ms=6000"));
        assert!(s.contains("retry_after_ms=Some(4800)"));
        assert!(s.contains("delivery=NotSent"));
    }

    #[test]
    fn recovery_info_peer_failed_display() {
        let peer = ActrId::default();
        let info = RecoveryInfo::new(
            peer.clone(),
            Some(7),
            RecoveryCode::PeerFailed,
            "peer state Failed",
            3500,
            6000,
        );
        let s = format!("{info}");
        assert!(s.starts_with("Connection recovering: peer="));
        assert!(s.contains("code=PeerFailed"));
        assert!(s.contains("reason=peer state Failed"));
        assert!(s.contains("elapsed_ms=3500"));
    }

    #[test]
    fn recovery_info_ice_network_started_display() {
        let peer = ActrId::default();
        let info = RecoveryInfo::new(
            peer.clone(),
            Some(99),
            RecoveryCode::IceNetworkStarted,
            "ice/network recovery started",
            0,
            6000,
        );
        let s = format!("{info}");
        assert!(s.contains("code=IceNetworkStarted"));
        assert!(s.contains("reason=ice/network recovery started"));
        assert!(s.contains("elapsed_ms=0"));
    }

    #[test]
    fn recovery_info_recovery_timeout_display() {
        let peer = ActrId::default();
        let info = RecoveryInfo::new(
            peer.clone(),
            Some(10),
            RecoveryCode::RecoveryTimeout,
            "ice restart failed",
            6000,
            6000,
        );
        let s = format!("{info}");
        assert!(s.starts_with("Connection recovery timeout:"));
        assert!(s.contains("code=RecoveryTimeout"));
        assert!(s.contains("reason=ice restart failed"));
        assert!(s.contains("elapsed_ms=6000"));
    }

    #[test]
    fn recovery_info_transport_closing_display() {
        let peer = ActrId::default();
        let info = RecoveryInfo::new(
            peer.clone(),
            None,
            RecoveryCode::TransportClosing,
            "transport closing",
            0,
            6000,
        );
        let s = format!("{info}");
        assert!(s.starts_with("Connection recovering: peer="));
        assert!(s.contains("session_id=None"));
        assert!(s.contains("code=TransportClosing"));
        assert!(s.contains("reason=transport closing"));
    }

    // ── Recovering classification ────────────────────────────────────────

    #[test]
    fn recovering_classifies_as_transient() {
        let peer = ActrId::default();
        let err = ActrError::Recovering(RecoveryInfo::new(
            peer,
            Some(1),
            RecoveryCode::PeerDisconnected,
            "peer state Disconnected",
            0,
            6000,
        ));
        assert_eq!(err.kind(), ErrorKind::Transient);
    }

    #[test]
    fn recovering_is_retryable() {
        let peer = ActrId::default();
        let err = ActrError::Recovering(RecoveryInfo::new(
            peer,
            None,
            RecoveryCode::TransportClosing,
            "transport closing",
            0,
            6000,
        ));
        assert!(err.is_retryable());
    }
}
