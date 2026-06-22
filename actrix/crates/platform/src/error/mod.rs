//! 错误处理模块
//!
//! 按概念分离的错误类型定义，遵循一个文件一个核心概念的原则

// 子模块声明
mod base_error;
mod config_error;
mod database_error;
mod network_error;
mod serialization_error;
mod storage_error;
mod validation_error;

// 导出公共 API
pub use base_error::{BaseError, Result};
pub use config_error::ConfigError;
pub use database_error::DatabaseError;
pub use network_error::NetworkError;
pub use serialization_error::SerializationError;
pub use storage_error::StorageError;
pub use validation_error::ValidationError;
