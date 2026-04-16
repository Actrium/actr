//! Shared user configuration file loader.

use super::schema::CliConfig;
use crate::error::{ConfigError, Result};
use std::path::{Path, PathBuf};

/// Returns the path to the global user-level config file: `~/.actr/config.toml`.
pub fn global_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        ConfigError::InvalidConfig("Unable to determine home directory".to_string())
    })?;
    Ok(home.join(".actr").join("config.toml"))
}

/// Returns the path to the local project-level config file: `.actr/config.toml`.
pub fn local_config_path() -> PathBuf {
    PathBuf::from(".actr").join("config.toml")
}

/// Load a user config from the given path.
///
/// Returns `None` if the file does not exist.
/// Returns an error if the file exists but cannot be parsed or fails validation.
pub fn load_cli_config(path: &Path) -> Result<Option<CliConfig>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ConfigError::InvalidConfig(format!(
                "Failed to read {}: {}",
                path.display(),
                error
            )));
        }
    };
    let config: CliConfig = toml::from_str(&content).map_err(|error| {
        ConfigError::InvalidConfig(format!("Failed to parse {}: {}", path.display(), error))
    })?;
    config.validate().map_err(|error| {
        ConfigError::InvalidConfig(format!("Invalid config at {}: {}", path.display(), error))
    })?;

    Ok(Some(config))
}
