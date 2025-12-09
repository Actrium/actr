//! Realm 错误类型定义
//!
//! 定义了 Realm 管理相关的错误类型

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RealmError {
    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Realm not found")]
    NotFound,

    #[error("Realm already exists")]
    AlreadyExists,

    #[error("Key expired")]
    KeyExpired,

    #[error("Key does not exist")]
    KeyNotExist,

    #[error("Parse error: {0}")]
    ParseError(String),
}

impl From<sqlx::Error> for RealmError {
    fn from(err: sqlx::Error) -> Self {
        RealmError::DatabaseError(err.to_string())
    }
}
