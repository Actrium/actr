//! Error types for actr-config

use thiserror::Error;

/// Errors that can occur during configuration parsing and processing
#[derive(Error, Debug)]
pub enum ActrConfigError {
    #[error("Failed to read configuration file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse TOML configuration: {0}")]
    TomlError(#[from] toml::de::Error),

    #[error("Invalid URL in configuration: {0}")]
    UrlError(#[from] url::ParseError),

    #[cfg(feature = "git-support")]
    #[error("Git operation failed: {0}")]
    GitError(#[from] git2::Error),

    #[error("Invalid proto dependency configuration: {0}")]
    InvalidDependency(String),

    #[error("Invalid routing rule: {0}")]
    InvalidRoutingRule(String),

    #[error("Missing required field in configuration: {0}")]
    MissingField(String),

    #[error("Configuration validation failed: {0}")]
    ValidationError(String),
}

pub type Result<T> = std::result::Result<T, ActrConfigError>;
