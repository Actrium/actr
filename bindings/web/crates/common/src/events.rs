//! SW <-> DOM communication event definitions
//!
//! Defines the message protocol between the Service Worker and the DOM.

use crate::{Dest, WebError, WebResult};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Connection type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnType {
    /// WebSocket connection.
    WebSocket,
    /// WebRTC connection.
    WebRTC,
}

/// SW -> DOM: request to create a P2P connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateP2PRequest {
    /// Target destination.
    pub dest: Dest,

    /// Request ID used to match the response.
    pub request_id: String,
}

impl CreateP2PRequest {
    /// Create a new request. The caller provides `request_id`.
    pub fn new(dest: Dest, request_id: String) -> Self {
        Self { dest, request_id }
    }

    /// Serialize the request.
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// Deserialize the request.
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
    }
}

/// DOM -> SW: P2P connection ready event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PReadyEvent {
    /// Matching request ID.
    pub request_id: String,

    /// Target destination.
    pub dest: Dest,

    /// Whether the operation succeeded.
    pub success: bool,

    /// Failure reason, if any.
    pub error_message: Option<String>,
}

impl P2PReadyEvent {
    /// Create a successful event.
    pub fn success(request_id: String, dest: Dest) -> Self {
        Self {
            request_id,
            dest,
            success: true,
            error_message: None,
        }
    }

    /// Create a failed event.
    pub fn failure(request_id: String, dest: Dest, error: String) -> Self {
        Self {
            request_id,
            dest,
            success: false,
            error_message: Some(error),
        }
    }

    /// Serialize the event.
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// Deserialize the event.
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
    }
}

/// Error severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ErrorSeverity {
    /// Warning: non-fatal, the system can continue running.
    Warning,
    /// Error: affects functionality, but the overall system remains usable.
    Error,
    /// Critical: a key function is broken and needs immediate attention.
    Critical,
    /// Fatal: the system cannot continue running.
    Fatal,
}

/// Error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCategory {
    /// WebRTC connection error.
    WebRTC,
    /// WebSocket connection error.
    WebSocket,
    /// MessagePort communication error.
    MessagePort,
    /// Data transport error.
    Transport,
    /// Serialization or deserialization error.
    Serialization,
    /// Timeout error.
    Timeout,
    /// Internal logic error.
    Internal,
}

/// DOM -> SW: error report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorReport {
    /// Error ID used for tracing.
    pub error_id: String,

    /// Error category.
    pub category: ErrorCategory,

    /// Error severity.
    pub severity: ErrorSeverity,

    /// Error message.
    pub message: String,

    /// Optional error context.
    pub context: Option<ErrorContext>,

    /// Timestamp in milliseconds.
    pub timestamp: f64,
}

/// Error context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Related destination, if any.
    pub dest: Option<Dest>,

    /// Related connection type, if any.
    pub conn_type: Option<ConnType>,

    /// Additional debug information.
    pub debug_info: Option<String>,
}

impl ErrorReport {
    /// Create a new error report.
    pub fn new(category: ErrorCategory, severity: ErrorSeverity, message: String) -> Self {
        use js_sys::Date;

        Self {
            error_id: Self::generate_error_id(),
            category,
            severity,
            message,
            context: None,
            timestamp: Date::now(),
        }
    }

    /// Attach context.
    pub fn with_context(mut self, context: ErrorContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Generate an error ID.
    fn generate_error_id() -> String {
        use js_sys::{Date, Math};
        format!(
            "err-{}-{}",
            Date::now() as u64,
            (Math::random() * 1_000_000.0) as u64
        )
    }

    /// Serialize the report.
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// Deserialize the report.
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
    }
}

/// SW/DOM control message type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlMessage {
    /// Request P2P creation.
    CreateP2P(CreateP2PRequest),

    /// P2P ready notification.
    P2PReady(P2PReadyEvent),

    /// Error report (DOM -> SW).
    ErrorReport(ErrorReport),
}

impl ControlMessage {
    /// Serialize the control message.
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// Deserialize the control message.
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    fn test_error_severity_ord() {
        assert!(ErrorSeverity::Warning < ErrorSeverity::Error);
        assert!(ErrorSeverity::Error < ErrorSeverity::Critical);
        assert!(ErrorSeverity::Critical < ErrorSeverity::Fatal);
    }

    #[test]
    fn test_error_category_variants() {
        let categories = vec![
            ErrorCategory::WebRTC,
            ErrorCategory::WebSocket,
            ErrorCategory::MessagePort,
            ErrorCategory::Transport,
            ErrorCategory::Serialization,
            ErrorCategory::Timeout,
            ErrorCategory::Internal,
        ];

        // Ensure every variant can be created correctly.
        for category in categories {
            assert_eq!(category, category);
        }
    }

    #[wasm_bindgen_test]
    fn test_error_report_creation() {
        let report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Test error".to_string(),
        );

        assert_eq!(report.category, ErrorCategory::WebRTC);
        assert_eq!(report.severity, ErrorSeverity::Error);
        assert_eq!(report.message, "Test error");
        assert!(report.error_id.starts_with("err-"));
        assert!(report.timestamp > 0.0);
        assert!(report.context.is_none());
    }

    #[wasm_bindgen_test]
    fn test_error_report_with_context() {
        let context = ErrorContext {
            dest: Some(Dest::Peer("peer1".to_string())),
            conn_type: Some(ConnType::WebRTC),
            debug_info: Some("Debug information".to_string()),
        };

        let report = ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Critical,
            "Critical error".to_string(),
        )
        .with_context(context.clone());

        assert_eq!(report.category, ErrorCategory::Transport);
        assert_eq!(report.severity, ErrorSeverity::Critical);
        assert!(report.context.is_some());

        let ctx = report.context.unwrap();
        assert_eq!(ctx.dest, Some(Dest::Peer("peer1".to_string())));
        assert_eq!(ctx.conn_type, Some(ConnType::WebRTC));
        assert_eq!(ctx.debug_info, Some("Debug information".to_string()));
    }

    #[wasm_bindgen_test]
    fn test_error_report_serialization() {
        let report = ErrorReport::new(
            ErrorCategory::WebSocket,
            ErrorSeverity::Warning,
            "Warning message".to_string(),
        );

        let serialized = report.serialize().expect("Serialization should succeed");
        let deserialized =
            ErrorReport::deserialize(&serialized).expect("Deserialization should succeed");

        assert_eq!(deserialized.category, report.category);
        assert_eq!(deserialized.severity, report.severity);
        assert_eq!(deserialized.message, report.message);
        assert_eq!(deserialized.error_id, report.error_id);
    }

    #[test]
    fn test_create_p2p_request() {
        let dest = Dest::Peer("peer1".to_string());
        let request = CreateP2PRequest::new(dest.clone(), "req-123".to_string());

        assert_eq!(request.dest, dest);
        assert_eq!(request.request_id, "req-123");
    }

    #[test]
    fn test_create_p2p_request_serialization() {
        let dest = Dest::Peer("peer1".to_string());
        let request = CreateP2PRequest::new(dest, "req-456".to_string());

        let serialized = request.serialize().expect("Serialization should succeed");
        let deserialized =
            CreateP2PRequest::deserialize(&serialized).expect("Deserialization should succeed");

        assert_eq!(deserialized.dest, request.dest);
        assert_eq!(deserialized.request_id, request.request_id);
    }

    #[test]
    fn test_p2p_ready_event_success() {
        let dest = Dest::Peer("peer1".to_string());
        let event = P2PReadyEvent::success("req-789".to_string(), dest.clone());

        assert_eq!(event.request_id, "req-789");
        assert_eq!(event.dest, dest);
        assert!(event.success);
        assert!(event.error_message.is_none());
    }

    #[test]
    fn test_p2p_ready_event_failure() {
        let dest = Dest::Peer("peer1".to_string());
        let event = P2PReadyEvent::failure(
            "req-999".to_string(),
            dest.clone(),
            "Connection failed".to_string(),
        );

        assert_eq!(event.request_id, "req-999");
        assert_eq!(event.dest, dest);
        assert!(!event.success);
        assert_eq!(event.error_message, Some("Connection failed".to_string()));
    }

    #[test]
    fn test_p2p_ready_event_serialization() {
        let dest = Dest::Peer("peer1".to_string());
        let event = P2PReadyEvent::success("req-111".to_string(), dest);

        let serialized = event.serialize().expect("Serialization should succeed");
        let deserialized =
            P2PReadyEvent::deserialize(&serialized).expect("Deserialization should succeed");

        assert_eq!(deserialized.request_id, event.request_id);
        assert_eq!(deserialized.dest, event.dest);
        assert_eq!(deserialized.success, event.success);
    }

    #[test]
    fn test_control_message_create_p2p() {
        let dest = Dest::Peer("peer1".to_string());
        let request = CreateP2PRequest::new(dest, "req-222".to_string());
        let msg = ControlMessage::CreateP2P(request);

        let serialized = msg.serialize().expect("Serialization should succeed");
        let deserialized =
            ControlMessage::deserialize(&serialized).expect("Deserialization should succeed");

        match deserialized {
            ControlMessage::CreateP2P(req) => {
                assert_eq!(req.request_id, "req-222");
            }
            _ => panic!("Expected CreateP2P variant"),
        }
    }

    #[test]
    fn test_control_message_p2p_ready() {
        let dest = Dest::Peer("peer1".to_string());
        let event = P2PReadyEvent::success("req-333".to_string(), dest);
        let msg = ControlMessage::P2PReady(event);

        let serialized = msg.serialize().expect("Serialization should succeed");
        let deserialized =
            ControlMessage::deserialize(&serialized).expect("Deserialization should succeed");

        match deserialized {
            ControlMessage::P2PReady(evt) => {
                assert_eq!(evt.request_id, "req-333");
                assert!(evt.success);
            }
            _ => panic!("Expected P2PReady variant"),
        }
    }

    #[wasm_bindgen_test]
    fn test_control_message_error_report() {
        let report = ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Error,
            "Transport error".to_string(),
        );
        let msg = ControlMessage::ErrorReport(report.clone());

        let serialized = msg.serialize().expect("Serialization should succeed");
        let deserialized =
            ControlMessage::deserialize(&serialized).expect("Deserialization should succeed");

        match deserialized {
            ControlMessage::ErrorReport(err) => {
                assert_eq!(err.category, ErrorCategory::Transport);
                assert_eq!(err.severity, ErrorSeverity::Error);
                assert_eq!(err.message, "Transport error");
            }
            _ => panic!("Expected ErrorReport variant"),
        }
    }

    #[wasm_bindgen_test]
    fn test_error_report_unique_ids() {
        let report1 = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Error 1".to_string(),
        );
        let report2 = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Error 2".to_string(),
        );

        // Error IDs should be unique.
        assert_ne!(report1.error_id, report2.error_id);
    }

    #[test]
    fn test_conn_type_equality() {
        assert_eq!(ConnType::WebSocket, ConnType::WebSocket);
        assert_eq!(ConnType::WebRTC, ConnType::WebRTC);
        assert_ne!(ConnType::WebSocket, ConnType::WebRTC);
    }
}
