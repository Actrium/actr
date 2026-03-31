//! CLI configuration file loader
//!
//! Handles locating and loading CLI configuration from the filesystem.

use super::schema::CliConfig;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Returns the path to the global user-level config file: ~/.actr/config.toml
pub fn global_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
    Ok(home.join(".actr").join("config.toml"))
}

/// Returns the path to the local project-level config file: .actr/config.toml
pub fn local_config_path() -> PathBuf {
    PathBuf::from(".actr").join("config.toml")
}

/// Load a CLI config from the given path.
///
/// Returns `None` if the file does not exist.
/// Returns an error if the file exists but cannot be parsed or fails validation.
pub fn load_cli_config(path: &Path) -> Result<Option<CliConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    let config: CliConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))?;
    config
        .validate()
        .map_err(|e| anyhow::anyhow!("Invalid config at {}: {}", path.display(), e))?;
    Ok(Some(config))
}
