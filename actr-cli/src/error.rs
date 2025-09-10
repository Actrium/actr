//! Error types for actr-cli

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ActrCliError {
    #[error("Configuration error: {0}")]
    Config(#[from] actr_config::ActrConfigError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Git operation failed: {0}")]
    Git(#[from] git2::Error),

    #[error("Cargo operation failed: {0}")]
    #[allow(dead_code)]
    Cargo(String),

    #[error("Template rendering failed: {0}")]
    Template(#[from] handlebars::RenderError),

    #[error("Project already exists: {0}")]
    ProjectExists(String),

    #[error("Invalid project structure: {0}")]
    InvalidProject(String),

    #[error("Build failed: {0}")]
    BuildFailed(String),

    #[error("Command execution failed: {0}")]
    CommandFailed(String),

    #[error("Proto dependency resolution failed: {0}")]
    ProtoDependency(String),
}

pub type Result<T> = std::result::Result<T, ActrCliError>;
