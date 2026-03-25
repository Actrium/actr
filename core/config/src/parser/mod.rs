//! Configuration parser - converts RawConfig to Config

use crate::error::{ConfigError, Result};
use crate::{Config, RawConfig};
use std::path::Path;

mod v1;

/// Configuration parser factory
pub struct ConfigParser;

impl ConfigParser {
    /// Select the appropriate parser based on edition and parse the config
    pub fn parse(raw: RawConfig, config_path: impl AsRef<Path>) -> Result<Config> {
        match raw.edition {
            1 => v1::ParserV1::new(config_path).parse(raw),
            // Future editions can be added here
            // 2 => v2::ParserV2::new(config_path).parse(raw),
            edition => Err(ConfigError::UnsupportedEdition(edition)),
        }
    }

    /// Load and parse config from file (convenience method)
    pub fn from_file(path: impl AsRef<Path>) -> Result<Config> {
        let raw = RawConfig::from_file(path.as_ref())?;
        Self::parse(raw, path)
    }
}
