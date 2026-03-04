//! DOM 侧错误报告器
//!
//! 负责将 DOM 侧的错误报告给 Service Worker

use crate::{WebError, WebResult};
use actr_web_common::{ConnType, Dest, ErrorCategory, ErrorContext, ErrorReport, ErrorSeverity};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::sync::Arc;
use web_sys::{MessagePort, Navigator, ServiceWorker, window};

/// DOM 错误报告器
pub struct DomErrorReporter {
    /// Service Worker 控制器（可选）
    sw_controller: Arc<Mutex<Option<ServiceWorker>>>,

    /// 控制通道 MessagePort（可选，用于直接通信）
    control_port: Arc<Mutex<Option<Arc<MessagePort>>>>,
}

impl DomErrorReporter {
    /// 创建新的错误报告器
    pub fn new() -> Self {
        Self {
            sw_controller: Arc::new(Mutex::new(None)),
            control_port: Arc::new(Mutex::new(None)),
        }
    }

    /// 初始化（尝试获取 SW 控制器）
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

    /// 设置控制通道
    pub fn set_control_port(&self, port: Arc<MessagePort>) {
        let mut control_port = self.control_port.lock();
        *control_port = Some(port);
        log::info!("[ErrorReporter] Control port registered");
    }

    /// 报告错误
    pub fn report_error(
        &self,
        category: ErrorCategory,
        severity: ErrorSeverity,
        message: String,
        context: Option<ErrorContext>,
    ) -> WebResult<()> {
        // 创建错误报告
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

        // 尝试发送到 SW
        self.send_to_sw(&report)?;

        Ok(())
    }

    /// 报告 WebRTC 错误
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

    /// 报告 MessagePort 错误
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

    /// 报告传输错误
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

    /// 发送到 SW
    fn send_to_sw(&self, report: &ErrorReport) -> WebResult<()> {
        // 序列化错误报告
        let data = report.serialize()?;

        // 优先使用控制通道
        if let Some(port) = self.control_port.lock().as_ref() {
            let js_array = js_sys::Uint8Array::from(data.as_ref());
            port.post_message(&js_array.into()).map_err(|e| {
                WebError::Transport(format!("Failed to send error via control port: {:?}", e))
            })?;

            log::debug!("[ErrorReporter] Error sent via control port");
            return Ok(());
        }

        // 备用：使用 SW controller
        if let Some(controller) = self.sw_controller.lock().as_ref() {
            // 包装成控制消息
            use actr_web_common::ControlMessage;
            let control_msg = ControlMessage::ErrorReport(report.clone());
            let msg_data = control_msg.serialize()?;

            // 将 Bytes 转换为 Vec<u8> 再序列化
            let data_vec: Vec<u8> = msg_data.to_vec();
            let js_value = serde_wasm_bindgen::to_value(&data_vec)
                .map_err(|e| WebError::Serialization(format!("Failed to serialize: {:?}", e)))?;

            controller.post_message(&js_value).map_err(|e| {
                WebError::Transport(format!("Failed to send error via SW controller: {:?}", e))
            })?;

            log::debug!("[ErrorReporter] Error sent via SW controller");
            return Ok(());
        }

        // 无可用通道
        log::warn!("[ErrorReporter] No channel available to send error");
        Err(WebError::Transport("No SW channel available".into()))
    }
}

impl Default for DomErrorReporter {
    fn default() -> Self {
        Self::new()
    }
}

/// 获取 Service Worker 控制器
fn get_sw_controller(navigator: &Navigator) -> Option<ServiceWorker> {
    navigator.service_worker().controller()
}

thread_local! {
    #[doc = "全局错误报告器实例（用于便捷访问）"]
    static GLOBAL_ERROR_REPORTER: RefCell<Option<Arc<DomErrorReporter>>> = const { RefCell::new(None) };
}

/// 初始化全局错误报告器
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

/// 获取全局错误报告器
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

        // 验证初始状态
        let sw_controller = reporter.sw_controller.lock();
        assert!(sw_controller.is_none());

        let control_port = reporter.control_port.lock();
        assert!(control_port.is_none());
    }

    #[wasm_bindgen_test]
    fn test_error_report_structure() {
        // 测试错误报告的构建
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

        // 验证所有严重级别都可以创建
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

        // 验证所有错误类别都可以创建
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

        // 每个错误报告应该有唯一的 ID
        assert_ne!(report1.error_id, report2.error_id);
    }

    #[wasm_bindgen_test]
    fn test_error_report_serialization_roundtrip() {
        let original = ErrorReport::new(
            ErrorCategory::MessagePort,
            ErrorSeverity::Error,
            "Serialization test".to_string(),
        );

        // 序列化
        let serialized = original.serialize().expect("Serialization should succeed");

        // 反序列化
        let deserialized =
            ErrorReport::deserialize(&serialized).expect("Deserialization should succeed");

        // 验证字段匹配
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

    // 注意：以下测试需要 wasm-bindgen-test 环境，在标准测试中会跳过

    #[wasm_bindgen_test]
    fn test_error_id_format() {
        let report = ErrorReport::new(
            ErrorCategory::Internal,
            ErrorSeverity::Error,
            "ID format test".to_string(),
        );

        // 错误 ID 应该以 "err-" 开头
        assert!(report.error_id.starts_with("err-"));

        // 错误 ID 应该包含时间戳和随机数
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

        // 时间戳应该是最近的（大于某个合理值）
        // 这里假设测试在 2020 年之后运行
        assert!(report.timestamp > 1_577_836_800_000.0); // 2020-01-01 的时间戳（毫秒）
    }

    #[wasm_bindgen_test]
    fn test_control_message_error_report_variant() {
        let report = ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Critical,
            "Control message test".to_string(),
        );

        let control_msg = ControlMessage::ErrorReport(report.clone());

        // 验证可以正确构造 ControlMessage
        match control_msg {
            ControlMessage::ErrorReport(r) => {
                assert_eq!(r.error_id, report.error_id);
                assert_eq!(r.message, report.message);
            }
            _ => panic!("Expected ErrorReport variant"),
        }
    }
}
