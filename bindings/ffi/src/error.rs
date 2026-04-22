//! Error types for actr UniFFI bindings.
//!
//! Mirrors `actr_protocol::ActrError` 1:1 so that every fault classification
//! made by the core framework survives the FFI boundary. A small number of
//! binding-local variants cover errors that occur strictly before a call
//! reaches the protocol layer (e.g. config parsing inside the shell).

use actr_protocol::{Classify, ErrorKind as ProtocolErrorKind};

/// Fault domain classification exposed to UniFFI consumers.
///
/// Mirrors `actr_protocol::ErrorKind` so downstream generic policy code
/// (retry / DLQ routing / alerting) can be written once and reused across
/// Swift, Kotlin, and any future UniFFI language target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ErrorKind {
    /// Environmental fluctuation — retry with exponential backoff.
    Transient,
    /// Caller error — bad request or system state; do not retry.
    Client,
    /// Framework bug or panic — do not retry; alert.
    Internal,
    /// Data corruption — route to Dead Letter Queue; manual intervention.
    Corrupt,
}

impl From<ProtocolErrorKind> for ErrorKind {
    fn from(k: ProtocolErrorKind) -> Self {
        match k {
            ProtocolErrorKind::Transient => ErrorKind::Transient,
            ProtocolErrorKind::Client => ErrorKind::Client,
            ProtocolErrorKind::Internal => ErrorKind::Internal,
            ProtocolErrorKind::Corrupt => ErrorKind::Corrupt,
        }
    }
}

/// Error type for actr operations.
///
/// The first ten variants mirror `actr_protocol::ActrError` exactly; the
/// remaining binding-local variants capture pre-protocol failures that
/// originate inside the FFI shell (config parsing, package loading, etc.).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ActrError {
    // ── Transient ─────────────────────────────────────────────────────────
    #[error("unavailable: {msg}")]
    Unavailable { msg: String },

    #[error("timed out")]
    TimedOut,

    // ── Client ────────────────────────────────────────────────────────────
    #[error("not found: {msg}")]
    NotFound { msg: String },

    #[error("permission denied: {msg}")]
    PermissionDenied { msg: String },

    #[error("invalid argument: {msg}")]
    InvalidArgument { msg: String },

    #[error("unknown route: {msg}")]
    UnknownRoute { msg: String },

    #[error("dependency '{service_name}' not found: {message}")]
    DependencyNotFound {
        service_name: String,
        message: String,
    },

    // ── Corrupt ───────────────────────────────────────────────────────────
    #[error("decode failure: {msg}")]
    DecodeFailure { msg: String },

    // ── Internal ──────────────────────────────────────────────────────────
    #[error("not implemented: {msg}")]
    NotImplemented { msg: String },

    #[error("internal error: {msg}")]
    Internal { msg: String },

    // ── FFI-local (pre-protocol) ──────────────────────────────────────────
    /// Config file parsing / trust resolution failed before the runtime could
    /// hand the request over to the protocol layer.
    ///
    /// Classified as `ErrorKind::Client` — the caller supplied a bad manifest
    /// or runtime config.
    #[error("config: {msg}")]
    Config { msg: String },
}

pub type ActrResult<T> = Result<T, ActrError>;

impl ActrError {
    /// Returns the fault domain this error belongs to.
    ///
    /// Exposed to UniFFI consumers so they can branch on classification
    /// rather than pattern-matching every variant.
    pub fn kind(&self) -> ErrorKind {
        match self {
            ActrError::Unavailable { .. } | ActrError::TimedOut => ErrorKind::Transient,

            ActrError::NotFound { .. }
            | ActrError::PermissionDenied { .. }
            | ActrError::InvalidArgument { .. }
            | ActrError::UnknownRoute { .. }
            | ActrError::DependencyNotFound { .. }
            | ActrError::Config { .. } => ErrorKind::Client,

            ActrError::DecodeFailure { .. } => ErrorKind::Corrupt,

            ActrError::NotImplemented { .. } | ActrError::Internal { .. } => ErrorKind::Internal,
        }
    }

    /// Returns `true` if the operation may be retried (Transient only).
    pub fn is_retryable(&self) -> bool {
        matches!(self.kind(), ErrorKind::Transient)
    }

    /// Returns `true` if the message should be routed to a Dead Letter Queue
    /// (Corrupt only).
    pub fn requires_dlq(&self) -> bool {
        matches!(self.kind(), ErrorKind::Corrupt)
    }
}

// UniFFI exposes errors as flat enum variants, not objects — so we cannot
// attach methods directly onto `ActrError`. Instead we expose classification
// helpers as free functions that take the error by reference-equivalent
// clone. Swift / Kotlin consumers call these as module-level functions:
// `actrErrorKind(err)` / `actrErrorIsRetryable(err)` / etc.

/// Fault-domain classification of `err` (see [`ErrorKind`]).
#[uniffi::export]
pub fn actr_error_kind(err: ActrError) -> ErrorKind {
    err.kind()
}

/// `true` iff the error is in the Transient fault domain — safe to retry.
#[uniffi::export]
pub fn actr_error_is_retryable(err: ActrError) -> bool {
    err.is_retryable()
}

/// `true` iff the error is in the Corrupt fault domain — route to DLQ.
#[uniffi::export]
pub fn actr_error_requires_dlq(err: ActrError) -> bool {
    err.requires_dlq()
}

impl From<actr_protocol::ActrError> for ActrError {
    fn from(e: actr_protocol::ActrError) -> Self {
        match e {
            actr_protocol::ActrError::Unavailable(msg) => ActrError::Unavailable { msg },
            actr_protocol::ActrError::TimedOut => ActrError::TimedOut,
            actr_protocol::ActrError::NotFound(msg) => ActrError::NotFound { msg },
            actr_protocol::ActrError::PermissionDenied(msg) => ActrError::PermissionDenied { msg },
            actr_protocol::ActrError::InvalidArgument(msg) => ActrError::InvalidArgument { msg },
            actr_protocol::ActrError::UnknownRoute(msg) => ActrError::UnknownRoute { msg },
            actr_protocol::ActrError::DependencyNotFound {
                service_name,
                message,
            } => ActrError::DependencyNotFound {
                service_name,
                message,
            },
            actr_protocol::ActrError::DecodeFailure(msg) => ActrError::DecodeFailure { msg },
            actr_protocol::ActrError::NotImplemented(msg) => ActrError::NotImplemented { msg },
            actr_protocol::ActrError::Internal(msg) => ActrError::Internal { msg },
        }
    }
}

impl From<ActrError> for actr_protocol::ActrError {
    fn from(e: ActrError) -> Self {
        match e {
            ActrError::Unavailable { msg } => actr_protocol::ActrError::Unavailable(msg),
            ActrError::TimedOut => actr_protocol::ActrError::TimedOut,
            ActrError::NotFound { msg } => actr_protocol::ActrError::NotFound(msg),
            ActrError::PermissionDenied { msg } => actr_protocol::ActrError::PermissionDenied(msg),
            ActrError::InvalidArgument { msg } => actr_protocol::ActrError::InvalidArgument(msg),
            ActrError::UnknownRoute { msg } => actr_protocol::ActrError::UnknownRoute(msg),
            ActrError::DependencyNotFound {
                service_name,
                message,
            } => actr_protocol::ActrError::DependencyNotFound {
                service_name,
                message,
            },
            ActrError::DecodeFailure { msg } => actr_protocol::ActrError::DecodeFailure(msg),
            ActrError::NotImplemented { msg } => actr_protocol::ActrError::NotImplemented(msg),
            ActrError::Internal { msg } => actr_protocol::ActrError::Internal(msg),
            // Binding-local Config has no direct protocol twin; classify as Client.
            ActrError::Config { msg } => actr_protocol::ActrError::InvalidArgument(msg),
        }
    }
}

/// Delegate to [`Classify`] on the protocol error for downstream crates that
/// want to treat this FFI error identically to the core error.
impl Classify for ActrError {
    fn kind(&self) -> ProtocolErrorKind {
        match self.kind() {
            ErrorKind::Transient => ProtocolErrorKind::Transient,
            ErrorKind::Client => ProtocolErrorKind::Client,
            ErrorKind::Internal => ProtocolErrorKind::Internal,
            ErrorKind::Corrupt => ProtocolErrorKind::Corrupt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_every_protocol_variant() {
        let cases = [
            actr_protocol::ActrError::Unavailable("u".into()),
            actr_protocol::ActrError::TimedOut,
            actr_protocol::ActrError::NotFound("nf".into()),
            actr_protocol::ActrError::PermissionDenied("pd".into()),
            actr_protocol::ActrError::InvalidArgument("ia".into()),
            actr_protocol::ActrError::UnknownRoute("ur".into()),
            actr_protocol::ActrError::DependencyNotFound {
                service_name: "svc".into(),
                message: "m".into(),
            },
            actr_protocol::ActrError::DecodeFailure("df".into()),
            actr_protocol::ActrError::NotImplemented("ni".into()),
            actr_protocol::ActrError::Internal("int".into()),
        ];

        for original in cases {
            let ffi: ActrError = original.clone().into();
            let back: actr_protocol::ActrError = ffi.into();
            assert_eq!(format!("{original}"), format!("{back}"));
        }
    }

    #[test]
    fn kind_classification_matches_protocol() {
        assert_eq!(
            ActrError::Unavailable { msg: "x".into() }.kind(),
            ErrorKind::Transient,
        );
        assert_eq!(ActrError::TimedOut.kind(), ErrorKind::Transient);
        assert_eq!(
            ActrError::NotFound { msg: "x".into() }.kind(),
            ErrorKind::Client,
        );
        assert_eq!(
            ActrError::DecodeFailure { msg: "x".into() }.kind(),
            ErrorKind::Corrupt,
        );
        assert_eq!(
            ActrError::Internal { msg: "x".into() }.kind(),
            ErrorKind::Internal,
        );
        assert_eq!(
            ActrError::Config { msg: "x".into() }.kind(),
            ErrorKind::Client,
        );
    }

    #[test]
    fn retry_and_dlq_predicates() {
        assert!(ActrError::Unavailable { msg: "x".into() }.is_retryable());
        assert!(!ActrError::NotFound { msg: "x".into() }.is_retryable());
        assert!(ActrError::DecodeFailure { msg: "x".into() }.requires_dlq());
        assert!(!ActrError::Internal { msg: "x".into() }.requires_dlq());
    }
}
