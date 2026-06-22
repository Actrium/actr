//! STUN 服务错误类型
//!
//! 定义 STUN 服务相关的所有错误类型

use thiserror::Error;

/// STUN 服务错误枚举
#[derive(Error, Debug)]
pub enum StunError {
    // ========== 服务器错误 ==========
    /// 服务器启动失败
    #[error("Failed to start STUN server: {reason}")]
    ServerStartFailed { reason: String },

    /// 服务器关闭失败
    #[error("Failed to shutdown STUN server: {reason}")]
    ServerShutdownFailed { reason: String },

    /// 端口绑定失败
    #[error("Failed to bind STUN server to port {port}: {reason}")]
    PortBindFailed { port: u16, reason: String },

    // ========== 协议错误 ==========
    /// 无效的 STUN 消息
    #[error("Invalid STUN message: {details}")]
    InvalidMessage { details: String },

    /// STUN 消息解析失败
    #[error("Failed to parse STUN message: {reason}")]
    MessageParseFailed { reason: String },

    /// STUN 消息编码失败
    #[error("Failed to encode STUN message: {reason}")]
    MessageEncodeFailed { reason: String },

    /// 不支持的消息类型
    #[error("Unsupported STUN message type: {message_type}")]
    UnsupportedMessageType { message_type: u16 },

    /// 无效的魔数
    #[error("Invalid STUN magic cookie")]
    InvalidMagicCookie,

    /// 消息长度错误
    #[error("Invalid message length: expected {expected}, got {actual}")]
    InvalidMessageLength { expected: usize, actual: usize },

    /// 无效的属性
    #[error("Invalid STUN attribute: {attribute_type}")]
    InvalidAttribute { attribute_type: u16 },

    // ========== 网络错误 ==========
    /// 网络连接错误
    #[error("Network connection error: {details}")]
    NetworkConnection { details: String },

    /// 数据包发送失败
    #[error("Failed to send STUN packet: {reason}")]
    PacketSendFailed { reason: String },

    /// 数据包接收失败
    #[error("Failed to receive STUN packet: {reason}")]
    PacketReceiveFailed { reason: String },

    /// 地址解析失败
    #[error("Address resolution failed: {address}")]
    AddressResolutionFailed { address: String },

    /// Socket 操作失败
    #[error("Socket operation failed: {operation}")]
    SocketOperationFailed { operation: String },

    // ========== 配置错误 ==========
    /// 配置错误
    #[error("Configuration error: {field} = {value}")]
    Configuration { field: String, value: String },

    /// 缺少必需的配置
    #[error("Missing required configuration: {field}")]
    MissingConfiguration { field: String },

    /// 无效的 IP 地址
    #[error("Invalid IP address: {address}")]
    InvalidIpAddress { address: String },

    /// 无效的端口
    #[error("Invalid port: {port}")]
    InvalidPort { port: String },

    // ========== 资源错误 ==========
    /// 资源耗尽
    #[error("Resource exhausted: {resource}")]
    ResourceExhausted { resource: String },

    /// 内存不足
    #[error("Out of memory")]
    OutOfMemory,

    /// 连接表已满
    #[error("Connection table full")]
    ConnectionTableFull,

    // ========== 超时错误 ==========
    /// 操作超时
    #[error("Operation timed out after {duration_ms}ms")]
    Timeout { duration_ms: u64 },

    /// 响应超时
    #[error("Response timeout for transaction {transaction_id}")]
    ResponseTimeout { transaction_id: String },

    // ========== 外部错误包装 ==========
    /// IO 错误
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// WebRTC 库错误
    #[error("WebRTC error: {0}")]
    WebRtc(#[from] webrtc::Error),

    /// 地址解析错误
    #[error("Address parse error: {0}")]
    AddrParse(#[from] std::net::AddrParseError),

    /// 通用错误
    #[error("General error: {message}")]
    General { message: String },
}

/// STUN 服务专用的 Result 类型
pub type Result<T> = std::result::Result<T, StunError>;

impl StunError {
    /// 创建通用错误
    pub fn general(message: impl Into<String>) -> Self {
        Self::General {
            message: message.into(),
        }
    }

    /// 创建消息解析错误
    pub fn parse_failed(reason: impl Into<String>) -> Self {
        Self::MessageParseFailed {
            reason: reason.into(),
        }
    }

    /// 创建配置错误
    pub fn config_error(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Configuration {
            field: field.into(),
            value: value.into(),
        }
    }

    /// 创建超时错误
    pub fn timeout(duration_ms: u64) -> Self {
        Self::Timeout { duration_ms }
    }

    /// 检查是否为可重试错误
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::NetworkConnection { .. }
            | Self::PacketSendFailed { .. }
            | Self::PacketReceiveFailed { .. }
            | Self::Timeout { .. }
            | Self::ResponseTimeout { .. }
            | Self::Io(_) => true,
            Self::ResourceExhausted { .. } | Self::OutOfMemory | Self::ConnectionTableFull => false,
            _ => false,
        }
    }

    /// 获取错误严重级别
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::ServerStartFailed { .. } | Self::ServerShutdownFailed { .. } => {
                ErrorSeverity::Critical
            }
            Self::InvalidMessage { .. }
            | Self::MessageParseFailed { .. }
            | Self::UnsupportedMessageType { .. } => ErrorSeverity::Warning,
            Self::Configuration { .. } | Self::MissingConfiguration { .. } => ErrorSeverity::Error,
            Self::ResourceExhausted { .. } | Self::OutOfMemory | Self::ConnectionTableFull => {
                ErrorSeverity::Critical
            }
            Self::Timeout { .. } | Self::ResponseTimeout { .. } => ErrorSeverity::Warning,
            _ => ErrorSeverity::Error,
        }
    }

    /// 检查是否为协议相关错误
    pub fn is_protocol_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidMessage { .. }
                | Self::MessageParseFailed { .. }
                | Self::MessageEncodeFailed { .. }
                | Self::UnsupportedMessageType { .. }
                | Self::InvalidMagicCookie
                | Self::InvalidMessageLength { .. }
                | Self::InvalidAttribute { .. }
        )
    }

    /// 检查是否为网络相关错误
    pub fn is_network_error(&self) -> bool {
        matches!(
            self,
            Self::NetworkConnection { .. }
                | Self::PacketSendFailed { .. }
                | Self::PacketReceiveFailed { .. }
                | Self::AddressResolutionFailed { .. }
                | Self::SocketOperationFailed { .. }
                | Self::Io(_)
                | Self::AddrParse(_)
        )
    }
}

/// 错误严重级别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// 信息级别
    Info,
    /// 警告级别
    Warning,
    /// 错误级别
    Error,
    /// 严重错误级别
    Critical,
}

// 为向后兼容性提供从 anyhow::Error 的转换
impl From<anyhow::Error> for StunError {
    fn from(err: anyhow::Error) -> Self {
        Self::general(err.to_string())
    }
}

// 从 webrtc-stun 库的错误转换
impl From<webrtc_stun::Error> for StunError {
    fn from(err: webrtc_stun::Error) -> Self {
        Self::MessageParseFailed {
            reason: err.to_string(),
        }
    }
}

// 为常见字符串类型提供转换
impl From<String> for StunError {
    fn from(message: String) -> Self {
        Self::general(message)
    }
}

impl From<&str> for StunError {
    fn from(message: &str) -> Self {
        Self::general(message.to_string())
    }
}

// 转换到统一的 BaseError
impl From<StunError> for platform::error::BaseError {
    fn from(err: StunError) -> Self {
        platform::error::BaseError::stun_service(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = StunError::general("test error");
        assert!(err.to_string().contains("test error"));
    }

    #[test]
    fn test_parse_error() {
        let err = StunError::parse_failed("invalid format");
        assert!(err.to_string().contains("invalid format"));
    }

    #[test]
    fn test_retryable_errors() {
        let timeout_err = StunError::timeout(5000);
        assert!(timeout_err.is_retryable());

        let config_err = StunError::Configuration {
            field: "port".to_string(),
            value: "invalid".to_string(),
        };
        assert!(!config_err.is_retryable());
    }

    #[test]
    fn test_error_severity() {
        let critical_err = StunError::ServerStartFailed {
            reason: "port in use".to_string(),
        };
        assert_eq!(critical_err.severity(), ErrorSeverity::Critical);

        let warning_err = StunError::InvalidMessage {
            details: "bad format".to_string(),
        };
        assert_eq!(warning_err.severity(), ErrorSeverity::Warning);
    }

    #[test]
    fn test_error_categories() {
        let protocol_err = StunError::InvalidMessage {
            details: "test".to_string(),
        };
        assert!(protocol_err.is_protocol_error());
        assert!(!protocol_err.is_network_error());

        let network_err = StunError::NetworkConnection {
            details: "test".to_string(),
        };
        assert!(!network_err.is_protocol_error());
        assert!(network_err.is_network_error());
    }
}
