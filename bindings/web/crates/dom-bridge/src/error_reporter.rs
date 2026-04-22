//! DOM-side error reporter
//!
//! Responsible for reporting DOM-side errors to the Service Worker

use crate::{WebError, WebResult};
use actr_web_common::{ConnType, Dest, ErrorCategory, ErrorContext, ErrorReport, ErrorSeverity};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::sync::Arc;
use web_sys::{MessagePort, Navigator, ServiceWorker, window};

/// DOM error reporter.
pub struct DomErrorReporter {
    /// Optional Service Worker controller.
    sw_controller: Arc<Mutex<Option<ServiceWorker>>>,

    /// Optional control-channel MessagePort for direct communication.
    control_port: Arc<Mutex<Option<Arc<MessagePort>>>>,
}

impl DomErrorReporter {
    /// Create a new error reporter.
    pub fn new() -> Self {
        Self {
            sw_controller: Arc::new(Mutex::new(None)),
            control_port: Arc::new(Mutex::new(None)),
        }
    }

    /// Initialize the reporter by trying to acquire the SW controller.
    pub fn init(&self) -> WebResult<()> {
        if let Some(win) = window() {
            let navigator = win.navigator();
            if let Some(controller) = get_sw_controller(&navigator) {
                let mut sw = self.sw_controller.lock();
                *sw = Some(controller);
                log::info!("[ErrorReporter] SW controller acquired");
            } else {
                log::warn!("[ErrorReporter] No SW controller found");
            }
        }
        Ok(())
    }

    /// Set the control port.
    pub fn set_control_port(&self, port: Arc<MessagePort>) {
        let mut control_port = self.control_port.lock();
        *control_port = Some(port);
        log::info!("[ErrorReporter] Control port registered");
    }

    /// Report an error.
    pub fn report_error(
        &self,
        category: ErrorCategory,
        severity: ErrorSeverity,
        message: String,
        context: Option<ErrorContext>,
    ) -> WebResult<()> {
        // Build the error report.
        let mut report = ErrorReport::new(category, severity, message);
        if let Some(ctx) = context {
            report = report.with_context(ctx);
        }

        log::error!(
            "[ErrorReporter] Reporting error: {:?} - {} (severity: {:?})",
            report.category,
            report.message,
            report.severity
        );

        // Attempt to send it to the Service Worker.
        self.send_to_sw(&report)?;

        Ok(())
    }

    /// Report a WebRTC error.
    pub fn report_webrtc_error(&self, dest: &Dest, message: String, severity: ErrorSeverity) {
        let context = ErrorContext {
            dest: Some(dest.clone()),
            conn_type: Some(ConnType::WebRTC),
            debug_info: None,
        };

        if let Err(e) = self.report_error(ErrorCategory::WebRTC, severity, message, Some(context)) {
            log::error!("[ErrorReporter] Failed to report WebRTC error: {:?}", e);
        }
    }

    /// Report a MessagePort error.
    pub fn report_messageport_error(&self, message: String, severity: ErrorSeverity) {
        let context = ErrorContext {
            dest: None,
            conn_type: None,
            debug_info: Some("MessagePort communication failure".to_string()),
        };

        if let Err(e) =
            self.report_error(ErrorCategory::MessagePort, severity, message, Some(context))
        {
            log::error!(
                "[ErrorReporter] Failed to report MessagePort error: {:?}",
                e
            );
        }
    }

    /// Report a transport error.
    pub fn report_transport_error(
        &self,
        conn_type: ConnType,
        message: String,
        severity: ErrorSeverity,
    ) {
        let context = ErrorContext {
            dest: None,
            conn_type: Some(conn_type),
            debug_info: None,
        };

        if let Err(e) =
            self.report_error(ErrorCategory::Transport, severity, message, Some(context))
        {
            log::error!("[ErrorReporter] Failed to report Transport error: {:?}", e);
        }
    }

    /// Send the report to the Service Worker.
    fn send_to_sw(&self, report: &ErrorReport) -> WebResult<()> {
        // Serialize the error report.
        let data = report.serialize()?;

        // Prefer the control channel when available.
        if let Some(port) = self.control_port.lock().as_ref() {
            let js_array = js_sys::Uint8Array::from(data.as_ref());
            port.post_message(&js_array.into()).map_err(|e| {
                WebError::Transport(format!("Failed to send error via control port: {:?}", e))
            })?;

            log::debug!("[ErrorReporter] Error sent via control port");
            return Ok(());
        }

        // Fallback: use the SW controller.
        if let Some(controller) = self.sw_controller.lock().as_ref() {
            // Wrap into a control message.
            use actr_web_common::ControlMessage;
            let control_msg = ControlMessage::ErrorReport(report.clone());
            let msg_data = control_msg.serialize()?;

            // Convert Bytes into Vec<u8>` before serializing to JsValue.
            let data_vec: Vec<u8> = msg_data.to_vec();
            let js_value = serde_wasm_bindgen::to_value(&data_vec)
                .map_err(|e| WebError::Serialization(format!("Failed to serialize: {:?}", e)))?;

            controller.post_message(&js_value).map_err(|e| {
                WebError::Transport(format!("Failed to send error via SW controller: {:?}", e))
            })?;

            log::debug!("[ErrorReporter] Error sent via SW controller");
            return Ok(());
        }

        // No channel is available.
        log::warn!("[ErrorReporter] No channel available to send error");
        Err(WebError::Transport("No SW channel available".into()))
    }
}

impl Default for DomErrorReporter {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the Service Worker controller.
fn get_sw_controller(navigator: &Navigator) -> Option<ServiceWorker> {
    navigator.service_worker().controller()
}

thread_local! {
    #[doc = "Global error reporter instance for convenient access."]
    static GLOBAL_ERROR_REPORTER: RefCell<Option<Arc<DomErrorReporter>>> = const { RefCell::new(None) };
}

/// Initialize the global error reporter.
pub fn init_global_error_reporter() -> Arc<DomErrorReporter> {
    GLOBAL_ERROR_REPORTER.with(|cell| {
        if let Some(reporter) = cell.borrow().as_ref() {
            return reporter.clone();
        }

        let reporter = Arc::new(DomErrorReporter::new());
        if let Err(e) = reporter.init() {
            log::error!("Failed to initialize error reporter: {:?}", e);
        }
        *cell.borrow_mut() = Some(reporter.clone());
        reporter
    })
}

/// Get the global error reporter.
pub fn get_global_error_reporter() -> Option<Arc<DomErrorReporter>> {
    GLOBAL_ERROR_REPORTER.with(|cell| cell.borrow().as_ref().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_web_common::ControlMessage;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_dom_error_reporter_creation() {
        let reporter = DomErrorReporter::new();

        // Verify the initial state.
        let sw_controller = reporter.sw_controller.lock();
        assert!(sw_controller.is_none());

        let control_port = reporter.control_port.lock();
        assert!(control_port.is_none());
    }

    #[wasm_bindgen_test]
    fn test_error_report_structure() {
        // Validate error report construction.
        let report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Test WebRTC error".to_string(),
        );

        assert_eq!(report.category, ErrorCategory::WebRTC);
        assert_eq!(report.severity, ErrorSeverity::Error);
        assert_eq!(report.message, "Test WebRTC error");
        assert!(report.error_id.starts_with("err-"));
        assert!(report.timestamp > 0.0);
    }

    #[wasm_bindgen_test]
    fn test_error_context_with_webrtc() {
        let dest = Dest::Peer("peer1".to_string());
        let context = ErrorContext {
            dest: Some(dest.clone()),
            conn_type: Some(ConnType::WebRTC),
            debug_info: Some("WebRTC connection failed".to_string()),
        };

        assert_eq!(context.dest, Some(dest));
        assert_eq!(context.conn_type, Some(ConnType::WebRTC));
        assert_eq!(
            context.debug_info,
            Some("WebRTC connection failed".to_string())
        );
    }

    #[wasm_bindgen_test]
    fn test_error_context_minimal() {
        let context = ErrorContext {
            dest: None,
            conn_type: None,
            debug_info: None,
        };

        assert!(context.dest.is_none());
        assert!(context.conn_type.is_none());
        assert!(context.debug_info.is_none());
    }

    #[wasm_bindgen_test]
    fn test_error_severity_levels() {
        let severities = vec![
            ErrorSeverity::Warning,
            ErrorSeverity::Error,
            ErrorSeverity::Critical,
            ErrorSeverity::Fatal,
        ];

        // Verify that all severity levels can be created.
        for severity in severities {
            let report = ErrorReport::new(
                ErrorCategory::Transport,
                severity,
                "Test message".to_string(),
            );
            assert_eq!(report.severity, severity);
        }
    }

    #[wasm_bindgen_test]
    fn test_error_categories() {
        let categories = vec![
            ErrorCategory::WebRTC,
            ErrorCategory::WebSocket,
            ErrorCategory::MessagePort,
            ErrorCategory::Transport,
            ErrorCategory::Serialization,
            ErrorCategory::Timeout,
            ErrorCategory::Internal,
        ];

        // Verify that all error categories can be created.
        for category in categories {
            let report =
                ErrorReport::new(category, ErrorSeverity::Error, "Test message".to_string());
            assert_eq!(report.category, category);
        }
    }

    #[wasm_bindgen_test]
    fn test_report_with_all_context_fields() {
        let context = ErrorContext {
            dest: Some(Dest::Peer("peer123".to_string())),
            conn_type: Some(ConnType::WebRTC),
            debug_info: Some("Full context test".to_string()),
        };

        let report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Critical,
            "Critical WebRTC error".to_string(),
        )
        .with_context(context);

        assert!(report.context.is_some());
        let ctx = report.context.unwrap();
        assert_eq!(ctx.dest, Some(Dest::Peer("peer123".to_string())));
        assert_eq!(ctx.conn_type, Some(ConnType::WebRTC));
        assert_eq!(ctx.debug_info, Some("Full context test".to_string()));
    }

    #[wasm_bindgen_test]
    fn test_multiple_error_reports_unique_ids() {
        let report1 = ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Warning,
            "First error".to_string(),
        );

        let report2 = ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Warning,
            "Second error".to_string(),
        );

        // Each error report should have a unique ID.
        assert_ne!(report1.error_id, report2.error_id);
    }

    #[wasm_bindgen_test]
    fn test_error_report_serialization_roundtrip() {
        let original = ErrorReport::new(
            ErrorCategory::MessagePort,
            ErrorSeverity::Error,
            "Serialization test".to_string(),
        );

        // Serialize.
        let serialized = original.serialize().expect("Serialization should succeed");

        // Deserialize.
        let deserialized =
            ErrorReport::deserialize(&serialized).expect("Deserialization should succeed");

        // Verify field equality.
        assert_eq!(deserialized.error_id, original.error_id);
        assert_eq!(deserialized.category, original.category);
        assert_eq!(deserialized.severity, original.severity);
        assert_eq!(deserialized.message, original.message);
        assert_eq!(deserialized.timestamp, original.timestamp);
    }

    #[wasm_bindgen_test]
    fn test_error_report_with_context_serialization() {
        let context = ErrorContext {
            dest: Some(Dest::Peer("test_peer".to_string())),
            conn_type: Some(ConnType::WebSocket),
            debug_info: Some("Debug data".to_string()),
        };

        let original = ErrorReport::new(
            ErrorCategory::WebSocket,
            ErrorSeverity::Warning,
            "Context serialization test".to_string(),
        )
        .with_context(context);

        let serialized = original.serialize().expect("Serialization should succeed");
        let deserialized =
            ErrorReport::deserialize(&serialized).expect("Deserialization should succeed");

        assert!(deserialized.context.is_some());
        let ctx = deserialized.context.unwrap();
        assert_eq!(ctx.dest, Some(Dest::Peer("test_peer".to_string())));
        assert_eq!(ctx.conn_type, Some(ConnType::WebSocket));
        assert_eq!(ctx.debug_info, Some("Debug data".to_string()));
    }

    #[wasm_bindgen_test]
    fn test_default_implementation() {
        let reporter = DomErrorReporter::default();

        let sw_controller = reporter.sw_controller.lock();
        assert!(sw_controller.is_none());

        let control_port = reporter.control_port.lock();
        assert!(control_port.is_none());
    }

    // Note: the following tests require wasm-bindgen-test and are skipped in standard test runs.

    #[wasm_bindgen_test]
    fn test_error_id_format() {
        let report = ErrorReport::new(
            ErrorCategory::Internal,
            ErrorSeverity::Error,
            "ID format test".to_string(),
        );

        // Error IDs should start with "err-".
        assert!(report.error_id.starts_with("err-"));

        // Error IDs should contain a timestamp and random suffix.
        let parts: Vec<&str> = report.error_id.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "err");
    }

    #[wasm_bindgen_test]
    fn test_timestamp_is_recent() {
        let report = ErrorReport::new(
            ErrorCategory::Timeout,
            ErrorSeverity::Error,
            "Timestamp test".to_string(),
        );

        // The timestamp should be recent and greater than a reasonable lower bound.
        // This assumes the test runs after 2020.
        assert!(report.timestamp > 1_577_836_800_000.0); // Timestamp for 2020-01-01 in milliseconds.
    }

    #[wasm_bindgen_test]
    fn test_control_message_error_report_variant() {
        let report = ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Critical,
            "Control message test".to_string(),
        );

        let control_msg = ControlMessage::ErrorReport(report.clone());

        // Verify that ControlMessage can be constructed correctly.
        match control_msg {
            ControlMessage::ErrorReport(r) => {
                assert_eq!(r.error_id, report.error_id);
                assert_eq!(r.message, report.message);
            }
            _ => panic!("Expected ErrorReport variant"),
        }
    }
}
