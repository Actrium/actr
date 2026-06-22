//! 配置相关错误类型
//!
//! 定义所有与配置解析、验证、加载相关的错误

use thiserror::Error;

/// 配置相关错误
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid configuration format: {message}")]
    InvalidFormat { message: String },

    #[error("Missing required field: {field}")]
    MissingField { field: String },

    #[error("Invalid value for field '{field}': {value}")]
    InvalidValue { field: String, value: String },

    #[error("Configuration file not found: {path}")]
    FileNotFound { path: String },

    #[error("Failed to parse configuration: {source}")]
    ParseError {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Environment variable error: {var}")]
    EnvError { var: String },
}
