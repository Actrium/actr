//! KS 服务错误定义

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

/// KS 服务错误类型
#[derive(Error, Debug)]
pub enum KsError {
    /// 数据库错误
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// 加密/解密错误
    #[error("Crypto error: {0}")]
    Crypto(String),

    /// 认证错误
    #[error("Authentication error: {0}")]
    Authentication(String),

    /// 重放攻击检测
    #[error("Replay attack detected: {0}")]
    ReplayAttack(String),

    /// 无效的请求参数
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// 密钥未找到
    #[error("Key not found: key_id={0}")]
    KeyNotFound(u32),

    /// 内部服务器错误
    #[error("Internal server error: {0}")]
    Internal(String),

    /// 配置错误
    #[error("Configuration error: {0}")]
    Config(String),

    /// Base64 编码/解码错误
    #[error("Base64 error: {0}")]
    Base64(#[from] base64::DecodeError),

    /// JSON 序列化/反序列化错误
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// HTTP 客户端错误
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),

    /// Nonce auth errors
    #[error("Nonce auth error: {0}")]
    NonceAuth(#[from] nonce_auth::NonceError),
}

impl IntoResponse for KsError {
    fn into_response(self) -> Response {
        let (status, error_message) = match &self {
            KsError::Authentication(_) => (
                StatusCode::UNAUTHORIZED,
                "Authentication failed".to_string(),
            ),
            KsError::ReplayAttack(_) => (StatusCode::FORBIDDEN, "Request rejected".to_string()),
            KsError::NonceAuth(_) => (
                StatusCode::UNAUTHORIZED,
                "Authentication failed".to_string(),
            ),
            KsError::InvalidRequest(_) => (
                StatusCode::BAD_REQUEST,
                "Invalid request parameters".to_string(),
            ),
            KsError::KeyNotFound(_) => {
                // 生产环境不泄露具体的 key_id
                (StatusCode::NOT_FOUND, "Resource not found".to_string())
            }
            KsError::Database(_) | KsError::Internal(_) | KsError::Crypto(_) => {
                // 不向客户端暴露内部错误详情
                tracing::error!("Internal error: {:?}", self);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            _ => {
                tracing::error!("Unexpected error: {:?}", self);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        let body = Json(json!({
            "error": error_message,
            "code": status.as_u16()
        }));

        (status, body).into_response()
    }
}

/// KS 结果类型别名
pub type KsResult<T> = Result<T, KsError>;

// ECIES 错误处理 - 由于 ecies 库的错误类型变化，暂时简化处理
// 后续可以根据具体需要添加更详细的错误转换
