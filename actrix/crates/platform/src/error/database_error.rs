//! 数据库相关错误类型
//!
//! 定义所有与数据库连接、查询、事务相关的错误

use thiserror::Error;

/// 数据库相关错误
#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Connection failed: {message}")]
    ConnectionFailed { message: String },

    #[error("Query failed: {query}")]
    QueryFailed { query: String },

    #[error("Transaction failed: {message}")]
    TransactionFailed { message: String },

    #[error("Migration failed: {version}")]
    MigrationFailed { version: String },

    #[error("Database not found: {name}")]
    NotFound { name: String },

    #[error("Constraint violation: {constraint}")]
    ConstraintViolation { constraint: String },

    #[error("SQLite error: {0}")]
    Sqlite(#[from] sqlx::Error),
}
