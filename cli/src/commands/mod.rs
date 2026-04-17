//! Command implementations for actr-cli.
//!
//! All commands implement [`crate::core::Command`]; dispatch happens in
//! `crate::cli::run`.

pub mod build;
pub mod check;
pub mod codegen;
pub mod completion;
pub mod config;
pub mod deps;
pub mod discovery;
pub mod dlq;
pub mod doc;
pub mod fingerprint;
pub mod generate;
pub mod init;
pub mod initialize;
pub mod install;
pub mod logs;
pub(crate) mod package_build;
pub mod pkg;
pub(crate) mod process;
pub mod ps;
pub mod registry;
pub mod restart;
pub mod rm;
pub mod run;
#[cfg(any(test, feature = "test-utils"))]
pub mod runtime_state;
#[cfg(not(any(test, feature = "test-utils")))]
pub(crate) mod runtime_state;
pub mod start;
pub mod stop;
pub mod version;

use clap::ValueEnum;

/// Supported languages for CLI commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum SupportedLanguage {
    Rust,
    Python,
    Swift,
    Kotlin,
    #[value(name = "typescript")]
    TypeScript,
}

pub use build::BuildCommand;
pub use check::CheckCommand;
pub use completion::CompletionCommand;
pub use config::ConfigCommand;
pub use deps::DepsArgs;
pub use dlq::DlqArgs;
pub use doc::DocCommand;
pub use generate::GenCommand;
pub use init::InitCommand;
pub use logs::LogsCommand;
pub use pkg::PkgArgs;
pub use ps::PsCommand;
pub use registry::RegistryArgs;
pub use restart::RestartCommand;
pub use rm::RmCommand;
pub use run::RunCommand;
pub use start::StartCommand;
pub use stop::StopCommand;
pub use version::VersionCommand;
