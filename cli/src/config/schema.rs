//! CLI configuration schema
//!
//! This module defines the configuration structures for CLI user preferences.

use serde::{Deserialize, Serialize};

/// CLI configuration file schema
///
/// Represents the structure of both ~/.actr/config.toml and .actr/config.toml.
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

    /// UI/Output settings
    #[serde(default)]
    pub ui: UiConfig,

    /// Service discovery settings (for CLI discovery commands)
    #[serde(default)]
    pub discovery: DiscoveryConfig,
}

impl CliConfig {
    /// Validate configuration values
    pub fn validate(&self) -> Result<(), String> {
        // Validate version
        if let Some(v) = self.version {
            if v != 1 {
                return Err(format!(
                    "Unsupported config version: {}. Supported version is 1",
                    v
                ));
            }
        }

        // Validate init.manufacturer
        if let Some(ref manufacturer) = self.init.manufacturer {
            if manufacturer.trim().is_empty() {
                return Err("init.manufacturer cannot be empty".to_string());
            }
        }

        // Validate codegen.language
        if let Some(ref language) = self.codegen.language {
            let valid_languages = ["rust", "typescript", "swift", "kotlin", "python", "web"];
            if !valid_languages.contains(&language.as_str()) {
                return Err(format!(
                    "codegen.language '{}' is invalid. Valid values: {}",
                    language,
                    valid_languages.join(", ")
                ));
            }
        }

        // Validate ui.format
        if let Some(ref format) = self.ui.format {
            let valid_formats = ["toml", "json", "yaml"];
            if !valid_formats.contains(&format.as_str()) {
                return Err(format!(
                    "ui.format '{}' is invalid. Valid values: {}",
                    format,
                    valid_formats.join(", ")
                ));
            }
        }

        // Validate ui.color
        if let Some(ref color) = self.ui.color {
            let valid_colors = ["auto", "always", "never"];
            if !valid_colors.contains(&color.as_str()) {
                return Err(format!(
                    "ui.color '{}' is invalid. Valid values: {}",
                    color,
                    valid_colors.join(", ")
                ));
            }
        }

        // Validate discovery.signaling_url
        if let Some(ref url) = self.discovery.signaling_url {
            if url.trim().is_empty() {
                return Err("discovery.signaling_url cannot be empty".to_string());
            }
            // Basic URL validation
            if !url.starts_with("ws://") && !url.starts_with("wss://") {
                return Err(format!(
                    "discovery.signaling_url '{}' must start with ws:// or wss://",
                    url
                ));
            }
        }

        // Validate discovery.ais_endpoint
        if let Some(ref url) = self.discovery.ais_endpoint {
            if url.trim().is_empty() {
                return Err("discovery.ais_endpoint cannot be empty".to_string());
            }
            // Basic URL validation
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!(
                    "discovery.ais_endpoint '{}' must start with http:// or https://",
                    url
                ));
            }
        }

        // Validate discovery.realm_id
        if let Some(realm_id) = self.discovery.realm_id {
            if realm_id == 0 {
                return Err("discovery.realm_id must be a positive integer".to_string());
            }
        }

        // Validate discovery.realm_secret
        if let Some(ref secret) = self.discovery.realm_secret {
            if secret.is_empty() {
                return Err(
                    "discovery.realm_secret cannot be empty string (omit the field instead)"
                        .to_string(),
                );
            }
        }

        Ok(())
    }
}

/// Project initialization settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct InitConfig {
    /// Default manufacturer for generated actor types (e.g., "acme")
    pub manufacturer: Option<String>,
}

/// Code generation settings
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

/// Package installation settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {
    /// Automatically generate/update lock file after installation
    pub auto_lock: Option<bool>,

    /// Prefer cached packages over re-downloading
    pub prefer_cache: Option<bool>,
}

/// Cache settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    /// Cache directory path (supports ~ expansion)
    pub dir: Option<String>,
}

/// UI/Output settings
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

/// Service discovery settings
///
/// These settings are used by CLI discovery commands (check, install, discovery)
/// to connect to signaling server and AIS for service discovery.
/// This is separate from runtime configuration (actr.toml).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryConfig {
    /// Signaling server URL for CLI discovery
    ///
    /// Default: ws://localhost:8081/signaling/ws
    pub signaling_url: Option<String>,

    /// AIS (Actor Identity Service) endpoint for CLI discovery
    ///
    /// Default: http://localhost:8081/ais
    pub ais_endpoint: Option<String>,

    /// Realm ID for CLI temporary actor registration
    ///
    /// No default value - must be explicitly configured
    pub realm_id: Option<u32>,

    /// Realm secret for authentication (optional)
    ///
    /// Only required if target realm has secret validation enabled
    pub realm_secret: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_config() {
        let config = CliConfig {
            version: Some(1),
            init: InitConfig {
                manufacturer: Some("acme".to_string()),
            },
            codegen: CodegenConfig {
                language: Some("rust".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_version() {
        let config = CliConfig {
            version: Some(2),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_manufacturer() {
        let config = CliConfig {
            init: InitConfig {
                manufacturer: Some("   ".to_string()),
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_language() {
        let config = CliConfig {
            codegen: CodegenConfig {
                language: Some("invalid".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_format() {
        let config = CliConfig {
            ui: UiConfig {
                format: Some("xml".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_color() {
        let config = CliConfig {
            ui: UiConfig {
                color: Some("blue".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_default_config() {
        let config = CliConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_all_valid_languages() {
        for lang in ["rust", "typescript", "swift", "kotlin", "python", "web"] {
            let config = CliConfig {
                codegen: CodegenConfig {
                    language: Some(lang.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(
                config.validate().is_ok(),
                "Language {} should be valid",
                lang
            );
        }
    }

    #[test]
    fn test_validate_all_valid_formats() {
        for format in ["toml", "json", "yaml"] {
            let config = CliConfig {
                ui: UiConfig {
                    format: Some(format.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(
                config.validate().is_ok(),
                "Format {} should be valid",
                format
            );
        }
    }

    #[test]
    fn test_validate_all_valid_colors() {
        for color in ["auto", "always", "never"] {
            let config = CliConfig {
                ui: UiConfig {
                    color: Some(color.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(config.validate().is_ok(), "Color {} should be valid", color);
        }
    }

    #[test]
    fn test_validate_valid_discovery_config() {
        let config = CliConfig {
            discovery: DiscoveryConfig {
                signaling_url: Some("ws://localhost:8081/signaling/ws".to_string()),
                ais_endpoint: Some("http://localhost:8081/ais".to_string()),
                realm_id: Some(1001),
                realm_secret: Some("secret".to_string()),
            },
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_signaling_url() {
        let config = CliConfig {
            discovery: DiscoveryConfig {
                signaling_url: Some("http://localhost:8081".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_ais_endpoint() {
        let config = CliConfig {
            discovery: DiscoveryConfig {
                ais_endpoint: Some("ws://localhost:8081".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_zero_realm_id() {
        let config = CliConfig {
            discovery: DiscoveryConfig {
                realm_id: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_realm_secret() {
        let config = CliConfig {
            discovery: DiscoveryConfig {
                realm_secret: Some("".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
