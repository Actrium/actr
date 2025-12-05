//! Runtime layer error definition
//!
//! Error handling follows microservices patterns rather than classic Actor supervision:
//! - Application errors: Propagated as RPC responses (handled by caller)
//! - Framework errors: Classified as Transient/Permanent/Poison
//!
//! See docs/3.11-production-readiness.zh.md for complete error handling strategy.

use thiserror::Error;

/// Runtime error types following gRPC-style classification
///
/// # Error Classification
///
/// - **Transient**: Temporary failures, safe to retry (UNAVAILABLE, DEADLINE_EXCEEDED)
/// - **Permanent**: Require system state fix, do NOT retry (NOT_FOUND, INVALID_ARGUMENT)
/// - **Poison**: Corrupted messages requiring manual intervention (decode failures)
///
/// # Design Philosophy
///
/// Unlike classic Actor systems (Erlang/Akka) that use Supervision trees,
/// Actor-RTC treats each Actr as a microservice unit:
/// - Caller controls retry logic (not framework)
/// - Explicit error propagation (not transparent restart)
/// - Dead Letter Queue for poison messages
#[derive(Error, Debug)]
pub enum RuntimeError {
    // ========== Transient Errors (Retryable) ==========
    /// Service temporarily unavailable (gRPC UNAVAILABLE)
    ///
    /// **Transient**: Connection lost, peer overloaded, temporary resource exhaustion
    /// **Caller should**: Retry with exponential backoff
    #[error("Service unavailable: {message}")]
    Unavailable {
        message: String,
        /// Optional target Actor ID
        target: Option<actr_protocol::ActrId>,
    },

    /// Request timeout exceeded (gRPC DEADLINE_EXCEEDED)
    ///
    /// **Transient**: Network delay, peer slow response
    /// **Caller should**: Retry with longer timeout or give up
    #[error("Deadline exceeded: {message}")]
    DeadlineExceeded { message: String, timeout_ms: u64 },

    // ========== Permanent Errors (Not Retryable) ==========
    /// Target Actor not found (gRPC NOT_FOUND)
    ///
    /// **Permanent**: Actor not registered in signaling server
    /// **Caller should**: NOT retry, perform service discovery first
    #[error("Actor not found: {actor_id:?}")]
    NotFound {
        actor_id: actr_protocol::ActrId,
        message: String,
    },

    /// Invalid argument provided (gRPC INVALID_ARGUMENT)
    ///
    /// **Permanent**: Malformed request, validation failure
    /// **Caller should**: NOT retry, fix the request
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    /// Precondition not met (gRPC FAILED_PRECONDITION)
    ///
    /// **Permanent**: System state incompatible with operation
    /// **Caller should**: NOT retry, fix system state first
    #[error("Failed precondition: {0}")]
    FailedPrecondition(String),

    /// Permission denied (gRPC PERMISSION_DENIED)
    ///
    /// **Permanent**: ACL check failed
    /// **Caller should**: NOT retry, check authorization
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    // ========== Poison Message Errors (DLQ) ==========
    /// Protobuf decode failure
    ///
    /// **Poison**: Corrupted message, cannot be processed
    /// **Framework**: Move to Dead Letter Queue, log raw bytes
    #[error("Protobuf decode failed: {message}")]
    DecodeFailure {
        message: String,
        /// Raw bytes for manual analysis
        raw_bytes: Option<Vec<u8>>,
    },

    // ========== Internal Errors ==========
    /// Internal framework error (gRPC INTERNAL)
    ///
    /// **Severity**: High - indicates framework bug or panic
    /// **Framework**: Log stack trace, capture panic info
    #[error("Internal error: {message}")]
    Internal {
        message: String,
        /// Panic info if caused by handler panic
        panic_info: Option<String>,
    },

    /// Mailbox operation error
    ///
    /// **Severity**: Critical - SQLite database issue
    /// **Framework**: Trigger alert, may need manual intervention
    #[error("Mailbox error: {0}")]
    MailboxError(String),

    // ========== Legacy Errors (To Be Migrated) ==========
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Initialization error
    #[error("Initialization error: {0}")]
    InitializationError(String),

    /// Shutdown error
    #[error("Shutdown error: {0}")]
    ShutdownError(String),

    /// IO Error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON Error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// Protocol Error
    #[error("Protocol error: {0}")]
    ProtocolError(#[from] actr_protocol::ProtocolError),

    /// Other error
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

impl From<crate::transport::error::NetworkError> for RuntimeError {
    fn from(err: crate::transport::error::NetworkError) -> Self {
        // Map network errors to appropriate RuntimeError variants
        use crate::transport::error::NetworkError;
        match err {
            // Transient errors
            NetworkError::ConnectionError(_)
            | NetworkError::SignalingError(_)
            | NetworkError::WebRtcError(_)
            | NetworkError::NetworkUnreachableError(_)
            | NetworkError::ResourceExhaustedError(_)
            | NetworkError::NatTraversalError(_)
            | NetworkError::IceError(_)
            | NetworkError::WebSocketError(_) => RuntimeError::Unavailable {
                message: err.to_string(),
                target: None,
            },

            // Timeout errors
            NetworkError::TimeoutError(_) => RuntimeError::DeadlineExceeded {
                message: err.to_string(),
                timeout_ms: 0,
            },

            // Not found errors
            NetworkError::ConnectionNotFound(_)
            | NetworkError::ChannelNotFound(_)
            | NetworkError::NoRoute(_) => RuntimeError::NotFound {
                actor_id: actr_protocol::ActrId::default(),
                message: err.to_string(),
            },

            // Invalid argument errors
            NetworkError::InvalidArgument(_) | NetworkError::InvalidOperation(_) => {
                RuntimeError::InvalidArgument(err.to_string())
            }

            // Permanent configuration errors
            NetworkError::ConfigurationError(_) => {
                RuntimeError::ConfigurationError(err.to_string())
            }

            // Permission errors
            NetworkError::AuthenticationError(_) | NetworkError::PermissionError(_) => {
                RuntimeError::PermissionDenied(err.to_string())
            }

            // Decode/encode failures → poison messages
            NetworkError::DeserializationError(msg) => RuntimeError::DecodeFailure {
                message: msg,
                raw_bytes: None,
            },

            // Other errors
            NetworkError::ProtocolError(_)
            | NetworkError::SerializationError(_)
            | NetworkError::DataChannelError(_)
            | NetworkError::BroadcastError(_)
            | NetworkError::DtlsError(_)
            | NetworkError::StunTurnError(_)
            | NetworkError::ServiceDiscoveryError(_)
            | NetworkError::NotImplemented(_)
            | NetworkError::ChannelClosed(_)
            | NetworkError::ConnectionClosed(_)
            | NetworkError::SendError(_)
            | NetworkError::IoError(_)
            | NetworkError::UrlParseError(_)
            | NetworkError::JsonError(_)
            | NetworkError::Other(_) => RuntimeError::Other(anyhow::anyhow!("{err}")),
        }
    }
}

impl RuntimeError {
    /// Error classification for retry decision
    ///
    /// Follows gRPC status code semantics:
    /// - Transient: Safe to retry (UNAVAILABLE, DEADLINE_EXCEEDED)
    /// - Permanent: Do NOT retry (NOT_FOUND, INVALID_ARGUMENT, etc.)
    /// - Poison: Needs manual intervention (DecodeFailure)
    pub fn classification(&self) -> ErrorClassification {
        match self {
            // Transient errors
            RuntimeError::Unavailable { .. } | RuntimeError::DeadlineExceeded { .. } => {
                ErrorClassification::Transient
            }

            // Permanent errors
            RuntimeError::NotFound { .. }
            | RuntimeError::InvalidArgument(_)
            | RuntimeError::FailedPrecondition(_)
            | RuntimeError::PermissionDenied(_)
            | RuntimeError::ConfigurationError(_)
            | RuntimeError::InitializationError(_) => ErrorClassification::Permanent,

            // Poison messages
            RuntimeError::DecodeFailure { .. } => ErrorClassification::Poison,

            // Internal errors (may be transient or permanent, depends on context)
            RuntimeError::Internal { .. } | RuntimeError::MailboxError(_) => {
                ErrorClassification::Internal
            }

            // Legacy errors - default to permanent
            RuntimeError::ShutdownError(_)
            | RuntimeError::IoError(_)
            | RuntimeError::JsonError(_)
            | RuntimeError::ProtocolError(_)
            | RuntimeError::Other(_) => ErrorClassification::Permanent,
        }
    }

    /// Check if error is retryable (Transient classification)
    ///
    /// Caller should use exponential backoff for retry.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.classification(),
            ErrorClassification::Transient | ErrorClassification::Internal
        )
    }

    /// Check if error requires Dead Letter Queue
    ///
    /// Poison messages cannot be processed and need manual intervention.
    pub fn requires_dlq(&self) -> bool {
        matches!(self.classification(), ErrorClassification::Poison)
    }

    /// Get gRPC-style status code name
    ///
    /// For logging and metrics (compatible with gRPC status codes).
    pub fn status_code(&self) -> &'static str {
        match self {
            RuntimeError::Unavailable { .. } => "UNAVAILABLE",
            RuntimeError::DeadlineExceeded { .. } => "DEADLINE_EXCEEDED",
            RuntimeError::NotFound { .. } => "NOT_FOUND",
            RuntimeError::InvalidArgument(_) => "INVALID_ARGUMENT",
            RuntimeError::FailedPrecondition(_) => "FAILED_PRECONDITION",
            RuntimeError::PermissionDenied(_) => "PERMISSION_DENIED",
            RuntimeError::DecodeFailure { .. } => "DATA_LOSS",
            RuntimeError::Internal { .. } => "INTERNAL",
            RuntimeError::MailboxError(_) => "INTERNAL",
            RuntimeError::ConfigurationError(_) => "FAILED_PRECONDITION",
            RuntimeError::InitializationError(_) => "FAILED_PRECONDITION",
            RuntimeError::ShutdownError(_) => "UNAVAILABLE",
            RuntimeError::IoError(_) => "INTERNAL",
            RuntimeError::JsonError(_) => "INTERNAL",
            RuntimeError::ProtocolError(_) => "INTERNAL",
            RuntimeError::Other(_) => "UNKNOWN",
        }
    }

    /// Get error severity (1-10, 10 is most critical)
    ///
    /// Used for alerting thresholds and monitoring.
    pub fn severity(&self) -> u8 {
        match self {
            // Critical: System cannot function
            RuntimeError::ConfigurationError(_) | RuntimeError::InitializationError(_) => 10,

            // High: Data loss or corruption
            RuntimeError::MailboxError(_) | RuntimeError::DecodeFailure { .. } => 9,

            // Medium-High: Internal errors, may indicate bugs
            RuntimeError::Internal { .. } => 8,

            // Medium: Access control
            RuntimeError::PermissionDenied(_) => 7,

            // Medium-Low: Client errors
            RuntimeError::NotFound { .. }
            | RuntimeError::InvalidArgument(_)
            | RuntimeError::FailedPrecondition(_) => 5,

            // Low: Transient failures
            RuntimeError::Unavailable { .. } | RuntimeError::DeadlineExceeded { .. } => 3,

            // Very Low: Expected errors
            RuntimeError::ShutdownError(_) => 2,

            // Minimal: Infrastructure
            RuntimeError::IoError(_) | RuntimeError::JsonError(_) => 1,

            // Unknown
            RuntimeError::ProtocolError(_) | RuntimeError::Other(_) => 4,
        }
    }

    /// Check if error requires system shutdown
    ///
    /// Only fatal configuration/initialization errors should shutdown.
    pub fn requires_system_shutdown(&self) -> bool {
        matches!(
            self,
            RuntimeError::ConfigurationError(_) | RuntimeError::InitializationError(_)
        )
    }

    /// Get error category for metrics
    ///
    /// Used in Prometheus labels: `errors_total{category="unavailable"}`
    pub fn category(&self) -> &'static str {
        match self {
            RuntimeError::Unavailable { .. } => "unavailable",
            RuntimeError::DeadlineExceeded { .. } => "timeout",
            RuntimeError::NotFound { .. } => "not_found",
            RuntimeError::InvalidArgument(_) => "invalid_argument",
            RuntimeError::FailedPrecondition(_) => "failed_precondition",
            RuntimeError::PermissionDenied(_) => "permission_denied",
            RuntimeError::DecodeFailure { .. } => "decode_failure",
            RuntimeError::Internal { .. } => "internal",
            RuntimeError::MailboxError(_) => "mailbox",
            RuntimeError::ConfigurationError(_) => "configuration",
            RuntimeError::InitializationError(_) => "initialization",
            RuntimeError::ShutdownError(_) => "shutdown",
            RuntimeError::IoError(_) => "io",
            RuntimeError::JsonError(_) => "json",
            RuntimeError::ProtocolError(_) => "protocol",
            RuntimeError::Other(_) => "other",
        }
    }
}

/// Error classification for retry decision
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClassification {
    /// Transient: Temporary failure, safe to retry
    Transient,
    /// Permanent: Requires system state fix, do NOT retry
    Permanent,
    /// Poison: Corrupted message, requires manual intervention (DLQ)
    Poison,
    /// Internal: Framework error, may be transient or permanent
    Internal,
}

/// Runtime result type
pub type RuntimeResult<T> = Result<T, RuntimeError>;
