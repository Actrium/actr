//! CLI configuration module
//!
//! This module provides configuration schema and resolution for the actr CLI tool.
//! It handles user preferences via ~/.actr/config.toml and .actr/config.toml files.

pub mod loader;
pub mod resolver;
pub mod schema;

pub use schema::{CacheConfig, CliConfig, CodegenConfig, InitConfig, InstallConfig, UiConfig};
pub use resolver::{resolve_effective_cli_config, EffectiveCliConfig};
