//! Command implementations for actr-cli

pub mod build;
pub mod check;
pub mod codegen;
pub mod config;
pub mod discovery;
pub mod dlq;
pub mod doc;
pub mod fingerprint;
pub mod generate;
pub mod init;
pub mod initialize;
pub mod install;
pub mod run;

use crate::error::Result;
use async_trait::async_trait;
use clap::ValueEnum;

// Legacy command trait for backward compatibility
#[async_trait]
pub trait Command {
    async fn execute(&self) -> Result<()>;
}

/// Supported languages for CLI commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum SupportedLanguage {
    Rust,
    Python,
    Swift,
    Kotlin,
    #[value(name = "typescript")]
    TypeScript,
    #[value(name = "web")]
    Web,
}

// Re-export new architecture commands
pub use check::CheckCommand;
pub use config::ConfigCommand;
pub use discovery::DiscoveryCommand;
pub use doc::DocCommand;
pub use fingerprint::FingerprintCommand;
pub use generate::GenCommand;
pub use init::InitCommand;
pub use install::InstallCommand;
pub use run::RunCommand;
