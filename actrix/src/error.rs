//! 统一错误处理模型
//!
//! 提供主应用 actrix 的顶层错误类型，聚合所有子模块和依赖 crate 的错误

use thiserror::Error;

/// 主应用的统一错误枚举
///
/// 聚合来自所有子模块和依赖 crate 的错误类型，提供类型安全的错误处理
#[derive(Debug, Error)]
pub enum Error {
    // ========== 配置相关错误 ==========
    /// 配置文件相关错误
    #[error("Configuration error: {0}")]
    Config(#[from] Box<dyn std::error::Error>),

    // ========== 基础库错误 ==========
    /// Base crate 聚合错误
    #[error("Base library error: {0}")]
    Base(Box<actrix_common::error::BaseError>),

    // ========== 服务相关错误 ==========
    // Identity 错误类型已删除，因为 AIS 服务已被移除
    // TODO: 新版 signaling crate 没有导出 SignalingError，暂时禁用
    // /// 信令服务错误
    // #[error("Signaling service error: {0}")]
    // Signaling(#[from] signaling::SignalingError),
    /// STUN 服务错误
    #[error("STUN service error: {0}")]
    Stun(#[from] stun::StunError),

    /// TURN 服务错误
    #[error("TURN service error: {0}")]
    Turn(#[from] turn::TurnError),

    // ========== 系统级错误 ==========
    /// I/O 操作错误
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// 序列化/反序列化错误
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// 网络错误
    #[error("Network error: {0}")]
    Network(#[from] tokio::task::JoinError),

    // ========== 业务逻辑错误 ==========
    /// 服务启动失败
    #[error("Service startup failed: {message}")]
    ServiceStartup { message: String },

    /// 服务配置验证失败  
    #[error("Service configuration validation failed: {message}")]
    ServiceValidation { message: String },

    // ========== 通用错误 ==========
    /// Anyhow 错误兼容层
    #[error("Legacy error: {0}")]
    Anyhow(#[from] anyhow::Error),

    /// 自定义错误消息
    #[error("Application error: {message}")]
    Custom { message: String },
}

impl From<actrix_common::error::BaseError> for Error {
    fn from(err: actrix_common::error::BaseError) -> Self {
        Error::Base(Box::new(err))
    }
}

/// 统一的 Result 类型
///
/// 为主应用提供统一的错误处理结果类型
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// 创建自定义错误
    pub fn custom(message: impl Into<String>) -> Self {
        Self::Custom {
            message: message.into(),
        }
    }

    /// 创建服务启动失败错误
    pub fn service_startup(message: impl Into<String>) -> Self {
        Self::ServiceStartup {
            message: message.into(),
        }
    }

    /// 创建服务配置验证失败错误
    pub fn service_validation(message: impl Into<String>) -> Self {
        Self::ServiceValidation {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = Error::custom("test error");
        assert!(matches!(err, Error::Custom { .. }));
    }
}
