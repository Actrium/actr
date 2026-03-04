//! 错误类型定义

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, CodegenError>;

#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    #[error("Proto 解析错误: {0}")]
    ProtoParseError(String),

    #[error("模板渲染错误: {0}")]
    TemplateError(String),

    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),

    #[error("配置错误: {0}")]
    ConfigError(String),

    #[error("文件未找到: {}", .0.display())]
    FileNotFound(PathBuf),

    #[error("无效的 proto 文件: {}", .0.display())]
    InvalidProtoFile(PathBuf),

    #[error("代码生成错误: {0}")]
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
