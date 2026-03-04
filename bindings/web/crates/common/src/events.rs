//! SW ↔ DOM 通信事件定义
//!
//! 定义 Service Worker 和 DOM 之间的消息协议

use crate::{Dest, WebError, WebResult};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// 连接类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnType {
    /// WebSocket 连接
    WebSocket,
    /// WebRTC 连接
    WebRTC,
}

/// SW → DOM: 请求创建 P2P 连接
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateP2PRequest {
    /// 目标 Dest
    pub dest: Dest,

    /// 请求 ID（用于匹配响应）
    pub request_id: String,
}

impl CreateP2PRequest {
    /// 创建新请求（request_id 由调用方生成）
    pub fn new(dest: Dest, request_id: String) -> Self {
        Self { dest, request_id }
    }

    /// 序列化
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// 反序列化
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
    }
}

/// DOM → SW: P2P 连接就绪事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PReadyEvent {
    /// 对应的请求 ID
    pub request_id: String,

    /// 目标 Dest
    pub dest: Dest,

    /// 是否成功
    pub success: bool,

    /// 失败原因（如果失败）
    pub error_message: Option<String>,
}

impl P2PReadyEvent {
    /// 创建成功事件
    pub fn success(request_id: String, dest: Dest) -> Self {
        Self {
            request_id,
            dest,
            success: true,
            error_message: None,
        }
    }

    /// 创建失败事件
    pub fn failure(request_id: String, dest: Dest, error: String) -> Self {
        Self {
            request_id,
            dest,
            success: false,
            error_message: Some(error),
        }
    }

    /// 序列化
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// 反序列化
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
    }
}

/// 错误严重级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ErrorSeverity {
    /// 警告：非致命错误，系统可继续运行
    Warning,
    /// 错误：影响功能，但系统整体可用
    Error,
    /// 严重：关键功能失效，需要立即处理
    Critical,
    /// 致命：系统无法继续运行
    Fatal,
}

/// 错误类别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCategory {
    /// WebRTC 连接错误
    WebRTC,
    /// WebSocket 连接错误
    WebSocket,
    /// MessagePort 通信错误
    MessagePort,
    /// 数据传输错误
    Transport,
    /// 序列化/反序列化错误
    Serialization,
    /// 超时错误
    Timeout,
    /// 内部逻辑错误
    Internal,
}

/// DOM → SW: 错误报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorReport {
    /// 错误 ID（用于追踪）
    pub error_id: String,

    /// 错误类别
    pub category: ErrorCategory,

    /// 错误严重级别
    pub severity: ErrorSeverity,

    /// 错误消息
    pub message: String,

    /// 错误上下文（可选）
    pub context: Option<ErrorContext>,

    /// 时间戳（毫秒）
    pub timestamp: f64,
}

/// 错误上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// 相关的 Dest（如果有）
    pub dest: Option<Dest>,

    /// 相关的连接类型（如果有）
    pub conn_type: Option<ConnType>,

    /// 额外的调试信息
    pub debug_info: Option<String>,
}

impl ErrorReport {
    /// 创建新的错误报告
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

    /// 添加上下文
    pub fn with_context(mut self, context: ErrorContext) -> Self {
        self.context = Some(context);
        self
    }

    /// 生成错误 ID
    fn generate_error_id() -> String {
        use js_sys::{Date, Math};
        format!(
            "err-{}-{}",
            Date::now() as u64,
            (Math::random() * 1_000_000.0) as u64
        )
    }

    /// 序列化
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// 反序列化
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
    }
}

/// SW/DOM 控制消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlMessage {
    /// 请求创建 P2P
    CreateP2P(CreateP2PRequest),

    /// P2P 就绪通知
    P2PReady(P2PReadyEvent),

    /// 错误报告（DOM → SW）
    ErrorReport(ErrorReport),
}

impl ControlMessage {
    /// 序列化
    pub fn serialize(&self) -> WebResult<Bytes> {
        serde_json::to_vec(self)
            .map(Bytes::from)
            .map_err(|e| WebError::Serialization(e.to_string()))
    }

    /// 反序列化
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

        // 确保所有变体都可以正确创建
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

        // 错误 ID 应该是唯一的
        assert_ne!(report1.error_id, report2.error_id);
    }

    #[test]
    fn test_conn_type_equality() {
        assert_eq!(ConnType::WebSocket, ConnType::WebSocket);
        assert_eq!(ConnType::WebRTC, ConnType::WebRTC);
        assert_ne!(ConnType::WebSocket, ConnType::WebRTC);
    }
}
