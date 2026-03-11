//! Error types.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, CodegenError>;

#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    #[error("Proto parse error: {0}")]
    ProtoParseError(String),

    #[error("Template rendering error: {0}")]
    TemplateError(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("File not found: {}", .0.display())]
    FileNotFound(PathBuf),

    #[error("Invalid proto file: {}", .0.display())]
    InvalidProtoFile(PathBuf),

    #[error("Code generation error: {0}")]
    GenerationError(String),
}

impl CodegenError {
    pub fn proto_parse<S: Into<String>>(msg: S) -> Self {
        Self::ProtoParseError(msg.into())
    }

    pub fn template<S: Into<String>>(msg: S) -> Self {
        Self::TemplateError(msg.into())
    }

    pub fn config<S: Into<String>>(msg: S) -> Self {
        Self::ConfigError(msg.into())
    }

    pub fn generation<S: Into<String>>(msg: S) -> Self {
        Self::GenerationError(msg.into())
    }
}
