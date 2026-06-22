//! 验证相关错误类型
//!
//! 定义所有与数据验证、权限检查、规则验证相关的错误

use thiserror::Error;

/// 验证相关错误
#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("Invalid input: {field}")]
    InvalidInput { field: String },

    #[error("Value out of range: {field} = {value}")]
    OutOfRange { field: String, value: String },

    #[error("Required field missing: {field}")]
    Required { field: String },

    #[error("Invalid format: {field}")]
    InvalidFormat { field: String },

    #[error("Authentication failed: {reason}")]
    Authentication { reason: String },

    #[error("Authorization failed: {reason}")]
    Authorization { reason: String },

    #[error("Rate limit exceeded: {limit}")]
    RateLimit { limit: String },
}
