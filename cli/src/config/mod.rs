//! CLI configuration module
//!
//! This module provides configuration schema and resolution for the actr CLI tool.
//! It handles user preferences via ~/.actr/config.toml and .actr/config.toml files.

pub mod loader;
pub mod resolver;
pub mod schema;

pub use resolver::{EffectiveCliConfig, resolve_effective_cli_config};
pub use schema::{CacheConfig, CliConfig, CodegenConfig, InstallConfig, MfrConfig, UiConfig};
