//! 统一的CLI错误类型系统
//!
//! 设计原则：
//! 1. 语义明确：每种错误类型都有明确的使用场景
//! 2. 避免重复：消除语义重叠的错误类型
//! 3. 层次分明：区分系统错误vs业务错误
//! 4. 易于调试：提供足够的上下文信息

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ActrCliError {
    // === 系统级错误 ===
    #[error("IO operation failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network request failed: {0}")]
    Network(#[from] reqwest::Error),

    #[error("JSON serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Git operation failed: {0}")]
    Git(#[from] git2::Error),

    // === 配置相关错误 ===
    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Invalid project structure: {0}")]
    InvalidProject(String),

    #[error("Project already exists: {0}")]
    ProjectExists(String),

    // === 依赖和构建错误 ===
    #[error("Dependency resolution failed: {0}")]
    Dependency(String),

    #[error("Build process failed: {0}")]
    Build(String),

    #[error("Code generation failed: {0}")]
    CodeGeneration(String),

    // === 模板和初始化错误 ===
    #[error("Template rendering failed: {0}")]
    Template(#[from] handlebars::RenderError),

    // === 命令执行错误 ===
    #[error("Command execution failed: {0}")]
    Command(String),

    // === 底层库错误的包装 ===
    #[error("Actor framework error: {0}")]
    Actor(#[from] actr::protocol::ActrError),

    #[error("URI parsing error: {0}")]
    UriParsing(#[from] actr::protocol::uri::ActrUriError),

    #[error("Configuration parsing error: {0}")]
    ConfigParsing(#[from] actr::config::ConfigError),

    // === 通用错误包装器 ===
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

// 错误类型转换辅助
impl ActrCliError {
    /// 将字符串转换为配置错误
    pub fn config_error(msg: impl Into<String>) -> Self {
        Self::Configuration(msg.into())
    }

    /// 将字符串转换为依赖错误
    pub fn dependency_error(msg: impl Into<String>) -> Self {
        Self::Dependency(msg.into())
    }

    /// 将字符串转换为构建错误
    pub fn build_error(msg: impl Into<String>) -> Self {
        Self::Build(msg.into())
    }

    /// 将字符串转换为命令执行错误
    pub fn command_error(msg: impl Into<String>) -> Self {
        Self::Command(msg.into())
    }

    /// 检查是否为配置相关错误
    pub fn is_config_error(&self) -> bool {
        matches!(
            self,
            Self::Configuration(_) | Self::ConfigParsing(_) | Self::InvalidProject(_)
        )
    }

    /// 检查是否为网络相关错误
    pub fn is_network_error(&self) -> bool {
        matches!(self, Self::Network(_))
    }

    /// 获取用户友好的错误提示
    pub fn user_hint(&self) -> Option<&str> {
        match self {
            Self::InvalidProject(_) => Some("💡 Use 'actr init' to initialize a new project"),
            Self::ProjectExists(_) => Some("💡 Use --force to overwrite existing project"),
            Self::Configuration(_) => Some("💡 Check your Actr.toml configuration file"),
            Self::Dependency(_) => Some("💡 Try 'actr install --force' to refresh dependencies"),
            Self::Build(_) => Some("💡 Check proto files and dependencies"),
            Self::Network(_) => Some("💡 Check your network connection and proxy settings"),
            _ => None,
        }
    }
}

/// CLI特定的Result类型
pub type Result<T> = std::result::Result<T, ActrCliError>;

// === 错误兼容性转换 ===
// 保证现有代码的兼容性，同时引导向新错误类型迁移
