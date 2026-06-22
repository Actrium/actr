//! 存储相关错误类型
//!
//! 定义所有与文件系统、存储后端相关的错误

use thiserror::Error;

/// 存储相关错误
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("File not found: {path}")]
    FileNotFound { path: String },

    #[error("Permission denied: {path}")]
    PermissionDenied { path: String },

    #[error("Disk full: {path}")]
    DiskFull { path: String },

    #[error("Corruption detected: {path}")]
    Corruption { path: String },

    #[error("Storage backend error: {backend}")]
    Backend { backend: String },
}
