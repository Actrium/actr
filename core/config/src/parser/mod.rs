//! Configuration parsers for manifest.toml and actr.toml

use crate::config::{ManifestConfig, PackageInfo, RuntimeConfig};
use crate::error::{ConfigError, Result};
use crate::{ManifestRawConfig, RuntimeRawConfig};
use std::path::Path;

mod v1;

/// Configuration parser factory
pub struct ConfigParser;

impl ConfigParser {
    /// Select the appropriate parser based on edition and parse manifest.toml.
    pub fn parse_manifest(
        raw: ManifestRawConfig,
        config_path: impl AsRef<Path>,
    ) -> Result<ManifestConfig> {
        match raw.edition {
            1 => v1::ParserV1::new(config_path).parse_manifest(raw),
            edition => Err(ConfigError::UnsupportedEdition(edition)),
        }
    }

    /// Load and parse manifest.toml from file.
    pub fn from_manifest_file(path: impl AsRef<Path>) -> Result<ManifestConfig> {
        let raw = ManifestRawConfig::from_file(path.as_ref())?;
        Self::parse_manifest(raw, path)
    }

    /// Parse actr.toml with externally provided package info.
    pub fn parse_runtime(
        raw: RuntimeRawConfig,
        actr_path: impl AsRef<Path>,
        package: PackageInfo,
        tags: Vec<String>,
    ) -> Result<RuntimeConfig> {
        match raw.edition {
            1 => v1::ParserV1::new(actr_path).parse_runtime(raw, package, tags),
            edition => Err(ConfigError::UnsupportedEdition(edition)),
        }
    }

    /// Load and parse actr.toml from file with externally provided package info.
    pub fn from_runtime_file(
        path: impl AsRef<Path>,
        package: PackageInfo,
        tags: Vec<String>,
    ) -> Result<RuntimeConfig> {
        let raw = RuntimeRawConfig::from_file(path.as_ref())?;
        Self::parse_runtime(raw, path, package, tags)
    }
}
