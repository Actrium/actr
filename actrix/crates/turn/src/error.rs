//! TURN 服务错误类型
//!
//! 定义 TURN 服务相关的所有错误类型

use thiserror::Error;

/// TURN 服务错误枚举
#[derive(Error, Debug)]
pub enum TurnError {
    // ========== 服务器错误 ==========
    /// 服务器启动失败
    #[error("Failed to start TURN server: {reason}")]
    ServerStartFailed { reason: String },

    /// 服务器关闭失败
    #[error("Failed to shutdown TURN server: {reason}")]
    ServerShutdownFailed { reason: String },

    /// 端口绑定失败
    #[error("Failed to bind TURN server to port {port}: {reason}")]
    PortBindFailed { port: u16, reason: String },

    // ========== 认证错误 ==========
    /// 认证失败
    #[error("TURN authentication failed for user '{username}': {reason}")]
    AuthenticationFailed { username: String, reason: String },

    /// 认证器创建失败
    #[error("Failed to create authenticator: {reason}")]
    AuthenticatorCreationFailed { reason: String },

    /// 无效的认证凭据
    #[error("Invalid credentials: {details}")]
    InvalidCredentials { details: String },

    /// Token 验证失败
    #[error("Token validation failed: {reason}")]
    TokenValidationFailed { reason: String },

    // ========== 协议错误 ==========
    /// 无效的 TURN 消息
    #[error("Invalid TURN message: {details}")]
    InvalidMessage { details: String },

    /// 不支持的消息类型
    #[error("Unsupported message type: {message_type}")]
    UnsupportedMessageType { message_type: String },

    /// 分配失败
    #[error("Allocation failed: {reason}")]
    AllocationFailed { reason: String },

    /// 权限错误
    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },

    // ========== 网络错误 ==========
    /// 网络连接错误
    #[error("Network connection error: {details}")]
    NetworkConnection { details: String },

    /// 数据包发送失败
    #[error("Failed to send packet: {reason}")]
    PacketSendFailed { reason: String },

    /// 数据包接收失败
    #[error("Failed to receive packet: {reason}")]
    PacketReceiveFailed { reason: String },

    /// 地址解析失败
    #[error("Address resolution failed: {address}")]
    AddressResolutionFailed { address: String },

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

    /// 端口范围无效
    #[error("Invalid port range: {range}")]
    InvalidPortRange { range: String },

    // ========== 资源错误 ==========
    /// 资源耗尽
    #[error("Resource exhausted: {resource}")]
    ResourceExhausted { resource: String },

    /// 分配表已满
    #[error("Allocation table full")]
    AllocationTableFull,

    /// 端口池耗尽
    #[error("Port pool exhausted")]
    PortPoolExhausted,

    /// 内存不足
    #[error("Out of memory")]
    OutOfMemory,

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

/// TURN 服务专用的 Result 类型
pub type Result<T> = std::result::Result<T, TurnError>;

impl TurnError {
    /// 创建通用错误
    pub fn general(message: impl Into<String>) -> Self {
        Self::General {
            message: message.into(),
        }
    }

    /// 创建认证失败错误
    pub fn auth_failed(username: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::AuthenticationFailed {
            username: username.into(),
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

    /// 检查是否为可重试错误
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::NetworkConnection { .. }
            | Self::PacketSendFailed { .. }
            | Self::PacketReceiveFailed { .. }
            | Self::Io(_) => true,
            Self::ResourceExhausted { .. }
            | Self::AllocationTableFull
            | Self::PortPoolExhausted
            | Self::OutOfMemory => false,
            _ => false,
        }
    }

    /// 获取错误严重级别
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::ServerStartFailed { .. } | Self::ServerShutdownFailed { .. } => {
                ErrorSeverity::Critical
            }
            Self::AuthenticationFailed { .. } | Self::PermissionDenied { .. } => {
                ErrorSeverity::Warning
            }
            Self::Configuration { .. } | Self::MissingConfiguration { .. } => ErrorSeverity::Error,
            Self::ResourceExhausted { .. }
            | Self::AllocationTableFull
            | Self::PortPoolExhausted
            | Self::OutOfMemory => ErrorSeverity::Critical,
            _ => ErrorSeverity::Error,
        }
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
impl From<anyhow::Error> for TurnError {
    fn from(err: anyhow::Error) -> Self {
        Self::general(err.to_string())
    }
}

// 为常见字符串类型提供转换
impl From<String> for TurnError {
    fn from(message: String) -> Self {
        Self::general(message)
    }
}

impl From<&str> for TurnError {
    fn from(message: &str) -> Self {
        Self::general(message.to_string())
    }
}

// 转换到统一的 BaseError
impl From<TurnError> for platform::error::BaseError {
    fn from(err: TurnError) -> Self {
        platform::error::BaseError::turn_service(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = TurnError::general("test error");
        assert!(err.to_string().contains("test error"));
    }

    #[test]
    fn test_auth_error() {
        let err = TurnError::auth_failed("user1", "invalid password");
        assert!(err.to_string().contains("user1"));
        assert!(err.to_string().contains("invalid password"));
    }

    #[test]
    fn test_retryable_errors() {
        let network_err = TurnError::NetworkConnection {
            details: "timeout".to_string(),
        };
        assert!(network_err.is_retryable());

        let config_err = TurnError::Configuration {
            field: "port".to_string(),
            value: "invalid".to_string(),
        };
        assert!(!config_err.is_retryable());
    }

    #[test]
    fn test_error_severity() {
        let critical_err = TurnError::ServerStartFailed {
            reason: "port in use".to_string(),
        };
        assert_eq!(critical_err.severity(), ErrorSeverity::Critical);

        let warning_err = TurnError::AuthenticationFailed {
            username: "user".to_string(),
            reason: "bad password".to_string(),
        };
        assert_eq!(warning_err.severity(), ErrorSeverity::Warning);
    }
}
