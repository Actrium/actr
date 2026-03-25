//! Storage layer error definitions

use thiserror::Error;

/// Storage layer error type
#[derive(Error, Debug)]
pub enum StorageError {
    /// Database connection error
    #[error("Database connection error: {0}")]
    ConnectionError(String),

    /// Query execution error
    #[error("Query execution error: {0}")]
    QueryError(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// Data integrity error
    #[error("Data integrity error: {0}")]
    IntegrityError(String),

    /// Concurrency conflict error
    #[error("Concurrency conflict: {0}")]
    ConcurrencyError(String),

    /// Resource not found error
    #[error("Resource not found: {0}")]
    NotFoundError(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Other error
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Storage layer result type
pub type StorageResult<T> = Result<T, StorageError>;

/// Convert actor error to storage error
impl From<actr_protocol::ActrError> for StorageError {
    fn from(err: actr_protocol::ActrError) -> Self {
        StorageError::Other(anyhow::anyhow!("Actor error: {err}"))
    }
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        match err {
            rusqlite::Error::SqliteFailure(sqlite_err, msg) => {
                let message = format!(
                    "SQLite error: {:?} - {}",
                    sqlite_err.code,
                    msg.unwrap_or_default()
                );
                match sqlite_err.code {
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                        StorageError::ConcurrencyError(message)
                    }
                    rusqlite::ErrorCode::ConstraintViolation => {
                        StorageError::IntegrityError(message)
                    }
                    rusqlite::ErrorCode::NotFound => StorageError::NotFoundError(message),
                    _ => StorageError::QueryError(message),
                }
            }
            _ => StorageError::QueryError(err.to_string()),
        }
    }
}
