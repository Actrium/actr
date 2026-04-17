//! Unified CLI error type system
//!
//! Design principles:
//! 1. Clear semantics: each error type has a well-defined use case
//! 2. No duplication: eliminate semantically overlapping error types
//! 3. Layered: distinguish system errors vs. business errors
//! 4. Easy to debug: provide sufficient context information

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ActrCliError {
    // === System-level errors ===
    #[error("IO operation failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network request failed: {0}")]
    Network(#[from] reqwest::Error),

    #[error("JSON serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Git operation failed: {0}")]
    Git(#[from] git2::Error),

    // === Configuration errors ===
    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Invalid project structure: {0}")]
    InvalidProject(String),

    #[error("Project already exists: {0}")]
    ProjectExists(String),

    // === Dependency and build errors ===
    #[error("Dependency resolution failed: {0}")]
    Dependency(String),

    #[error("Build process failed: {0}")]
    Build(String),

    #[error("Code generation failed: {0}")]
    CodeGeneration(String),

    // === Template and initialization errors ===
    #[error("Template rendering failed: {0}")]
    Template(#[from] handlebars::RenderError),

    #[error("Unsupported feature: {0}")]
    Unsupported(String),

    // === Command execution errors ===
    #[error("Command execution failed: {0}")]
    Command(String),

    // === Wrapper for underlying library errors ===
    #[error("Actor framework error: {0}")]
    Actor(#[from] actr_protocol::ActrError),

    #[error("URI parsing error: {0}")]
    UriParsing(#[from] actr_protocol::uri::ActrUriError),

    #[error("Configuration parsing error: {0}")]
    ConfigParsing(#[from] actr_config::ConfigError),

    // === Generic error wrapper ===
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

// Error type conversion helpers
impl ActrCliError {
    /// Convert a string into a configuration error
    pub fn config_error(msg: impl Into<String>) -> Self {
        Self::Configuration(msg.into())
    }

    /// Convert a string into a dependency error
    pub fn dependency_error(msg: impl Into<String>) -> Self {
        Self::Dependency(msg.into())
    }

    /// Convert a string into a build error
    pub fn build_error(msg: impl Into<String>) -> Self {
        Self::Build(msg.into())
    }

    /// Convert a string into a command execution error
    pub fn command_error(msg: impl Into<String>) -> Self {
        Self::Command(msg.into())
    }

    /// Check whether this is a configuration-related error
    pub fn is_config_error(&self) -> bool {
        matches!(
            self,
            Self::Configuration(_) | Self::ConfigParsing(_) | Self::InvalidProject(_)
        )
    }

    /// Check whether this is a network-related error
    pub fn is_network_error(&self) -> bool {
        matches!(self, Self::Network(_))
    }

    /// Get a user-friendly hint for this error
    pub fn user_hint(&self) -> Option<&str> {
        match self {
            Self::InvalidProject(_) => Some("💡 Use 'actr init' to initialize a new project"),
            Self::ProjectExists(_) => Some("💡 Use --force to overwrite existing project"),
            Self::Configuration(_) => Some("💡 Check your manifest.toml configuration file"),
            Self::Dependency(_) => {
                Some("💡 Try 'actr deps install --force' to refresh dependencies")
            }
            Self::Build(_) => Some("💡 Check proto files and dependencies"),
            Self::Network(_) => Some("💡 Check your network connection and proxy settings"),
            Self::Unsupported(_) => Some("💡 This feature is not implemented yet"),
            _ => None,
        }
    }
}

/// CLI-specific Result type
pub type Result<T> = std::result::Result<T, ActrCliError>;

// === Error compatibility conversions ===
// Ensure backward compatibility while guiding migration to new error types
