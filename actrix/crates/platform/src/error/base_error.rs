//! 顶层错误枚举
//!
//! 聚合所有子模块的错误类型，提供统一的错误处理接口

use super::{
    ConfigError, DatabaseError, NetworkError, SerializationError, StorageError, ValidationError,
};
use thiserror::Error;

/// 顶层错误枚举，聚合所有子 crate 的错误
#[derive(Error, Debug)]
pub enum BaseError {
    // ========== Identity 管理错误 ==========
    /// AId Token 相关错误
    #[error("AId error: {0}")]
    Aid(#[from] crate::aid::credential::AidError),

    /// Realm 管理错误
    #[error("Realm error: {0}")]
    Realm(#[from] crate::realm::RealmError),

    // ========== 服务层错误 ==========
    /// 身份服务错误
    #[error("Identity service error: {message}")]
    IdentityService { message: String },

    /// 信令服务错误
    #[error("Signaling service error: {message}")]
    SignalingService { message: String },

    /// 监管服务错误
    #[error("Supervisor service error: {message}")]
    SupervisorService { message: String },

    /// TURN 服务错误
    #[error("TURN service error: {message}")]
    TurnService { message: String },

    /// STUN 服务错误
    #[error("STUN service error: {message}")]
    StunService { message: String },

    // ========== 基础设施错误 ==========
    /// 配置错误
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// 网络错误
    #[error("Network error: {0}")]
    Network(#[from] NetworkError),

    /// 数据库错误
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    /// 存储错误
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    // ========== 通用错误 ==========
    /// IO 错误
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// 序列化错误
    #[error("Serialization error: {0}")]
    Serialization(#[from] SerializationError),

    /// 验证错误
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),

    /// 通用错误（用于不适合其他类别的错误）
    #[error("General error: {message}")]
    General { message: String },

    /// 内部错误（通常表示编程错误）
    #[error("Internal error: {message}")]
    Internal { message: String },
}

impl BaseError {
    /// 创建通用错误
    pub fn general(message: impl Into<String>) -> Self {
        Self::General {
            message: message.into(),
        }
    }

    /// 创建内部错误
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// 创建身份服务错误
    pub fn identity_service(message: impl Into<String>) -> Self {
        Self::IdentityService {
            message: message.into(),
        }
    }

    /// 创建信令服务错误
    pub fn signaling_service(message: impl Into<String>) -> Self {
        Self::SignalingService {
            message: message.into(),
        }
    }

    /// 创建监管服务错误
    pub fn supervisor_service(message: impl Into<String>) -> Self {
        Self::SupervisorService {
            message: message.into(),
        }
    }

    /// 创建 TURN 服务错误
    pub fn turn_service(message: impl Into<String>) -> Self {
        Self::TurnService {
            message: message.into(),
        }
    }

    /// 创建 STUN 服务错误
    pub fn stun_service(message: impl Into<String>) -> Self {
        Self::StunService {
            message: message.into(),
        }
    }
}

/// 统一的 Result 类型
pub type Result<T> = std::result::Result<T, BaseError>;
