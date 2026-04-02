//! Shared CLI/user configuration schema.

use crate::error::{ConfigError, Result};
use serde::{Deserialize, Serialize};

/// User configuration file schema.
///
/// Represents the structure of both `~/.actr/config.toml` and `.actr/config.toml`.
/// All fields are optional to allow partial overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CliConfig {
    /// Config file format version (for future migration)
    pub version: Option<u32>,

    /// Project initialization settings
    #[serde(default)]
    pub init: InitConfig,

    /// Code generation settings
    #[serde(default)]
    pub codegen: CodegenConfig,

    /// Package installation settings
    #[serde(default)]
    pub install: InstallConfig,

    /// Cache settings
    #[serde(default)]
    pub cache: CacheConfig,

    /// UI/output settings
    #[serde(default)]
    pub ui: UiConfig,

    /// Network settings for CLI service discovery and connectivity checks
    #[serde(default)]
    pub network: NetworkConfig,

    /// Storage settings
    #[serde(default)]
    pub storage: StorageConfig,
}

impl CliConfig {
    /// Validate configuration values.
    pub fn validate(&self) -> Result<()> {
        if let Some(v) = self.version {
            if v != 1 {
                return Err(ConfigError::ValidationError(format!(
                    "Unsupported config version: {}. Supported version is 1",
                    v
                )));
            }
        }

        if let Some(ref manufacturer) = self.init.manufacturer {
            if manufacturer.trim().is_empty() {
                return Err(ConfigError::ValidationError(
                    "init.manufacturer cannot be empty".to_string(),
                ));
            }
        }

        if let Some(ref language) = self.codegen.language {
            let valid_languages = ["rust", "typescript", "swift", "kotlin", "python", "web"];
            if !valid_languages.contains(&language.as_str()) {
                return Err(ConfigError::ValidationError(format!(
                    "codegen.language '{}' is invalid. Valid values: {}",
                    language,
                    valid_languages.join(", ")
                )));
            }
        }

        if let Some(ref format) = self.ui.format {
            let valid_formats = ["toml", "json", "yaml"];
            if !valid_formats.contains(&format.as_str()) {
                return Err(ConfigError::ValidationError(format!(
                    "ui.format '{}' is invalid. Valid values: {}",
                    format,
                    valid_formats.join(", ")
                )));
            }
        }

        if let Some(ref color) = self.ui.color {
            let valid_colors = ["auto", "always", "never"];
            if !valid_colors.contains(&color.as_str()) {
                return Err(ConfigError::ValidationError(format!(
                    "ui.color '{}' is invalid. Valid values: {}",
                    color,
                    valid_colors.join(", ")
                )));
            }
        }

        if let Some(ref url) = self.network.signaling_url {
            if url.trim().is_empty() {
                return Err(ConfigError::ValidationError(
                    "network.signaling_url cannot be empty".to_string(),
                ));
            }
            if !url.starts_with("ws://") && !url.starts_with("wss://") {
                return Err(ConfigError::ValidationError(format!(
                    "network.signaling_url '{}' must start with ws:// or wss://",
                    url
                )));
            }
        }

        if let Some(ref url) = self.network.ais_endpoint {
            if url.trim().is_empty() {
                return Err(ConfigError::ValidationError(
                    "network.ais_endpoint cannot be empty".to_string(),
                ));
            }
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ConfigError::ValidationError(format!(
                    "network.ais_endpoint '{}' must start with http:// or https://",
                    url
                )));
            }
        }

        if let Some(realm_id) = self.network.realm_id {
            if realm_id == 0 {
                return Err(ConfigError::ValidationError(
                    "network.realm_id must be a positive integer".to_string(),
                ));
            }
        }

        if let Some(ref secret) = self.network.realm_secret {
            if secret.is_empty() {
                return Err(ConfigError::ValidationError(
                    "network.realm_secret cannot be empty string (omit the field instead)"
                        .to_string(),
                ));
            }
        }

        Ok(())
    }
}

/// Project initialization settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct InitConfig {
    /// Default manufacturer for generated actor types (e.g., "acme")
    pub manufacturer: Option<String>,
}

/// Code generation settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CodegenConfig {
    /// Default target language for code generation
    pub language: Option<String>,

    /// Default output directory for generated code
    pub output: Option<String>,

    /// Clean output directory before generating code
    pub clean_before_generate: Option<bool>,
}

/// Package installation settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {}

/// Cache settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    /// Cache directory path (supports `~` expansion)
    pub dir: Option<String>,

    /// Automatically generate/update lock file after installation
    pub auto_lock: Option<bool>,

    /// Prefer cached packages over re-downloading
    pub prefer_cache: Option<bool>,
}

/// UI/output settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    /// Output format for structured commands: "toml", "json", "yaml"
    pub format: Option<String>,

    /// Verbose output
    pub verbose: Option<bool>,

    /// Color output: "auto", "always", "never"
    pub color: Option<String>,

    /// Non-interactive mode (skip prompts)
    pub non_interactive: Option<bool>,
}

/// Network settings used by CLI/user-facing connectivity operations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// Signaling server URL for CLI discovery
    pub signaling_url: Option<String>,

    /// AIS endpoint for CLI discovery
    pub ais_endpoint: Option<String>,

    /// Realm ID for CLI temporary actor registration
    pub realm_id: Option<u32>,

    /// Realm secret for authentication (optional)
    pub realm_secret: Option<String>,
}

/// Storage settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Global Hyper data directory path (supports `~` expansion).
    pub hyper_data_dir: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_storage_hyper_dir() {
        let config = CliConfig {
            storage: StorageConfig {
                hyper_data_dir: Some("~/.actr/hyper".to_string()),
            },
            ..Default::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_invalid_version() {
        let config = CliConfig {
            version: Some(2),
            ..Default::default()
        };

        assert!(config.validate().is_err());
    }
}
