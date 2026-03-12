//! Config command implementation - manage project configuration
//!
//! Supports the following operations:
//! - `actr config set <key> <value>` - Set a configuration value
//! - `actr config get <key>` - Get a configuration value  
//! - `actr config list` - List all configuration keys
//! - `actr config show` - Show full configuration
//! - `actr config unset <key>` - Remove a configuration value
//! - `actr config test` - Test configuration file syntax

use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use actr_config::ConfigParser;
use actr_config::RawConfig;
use actr_protocol::ActrTypeExt;
use anyhow::{Result, bail};
use async_trait::async_trait;
use clap::{Args, Subcommand};
use owo_colors::OwoColorize;
use std::path::Path;

#[derive(Args, Clone)]
pub struct ConfigCommand {
    /// Configuration file to load (defaults to actr.toml)
    #[arg(short = 'f', long = "file")]
    pub config_file: Option<String>,

    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Subcommand, Clone)]
pub enum ConfigSubcommand {
    /// Set a configuration value
    Set {
        /// Configuration key (e.g., "signaling.url", "build.output-dir")
        key: String,
        /// Configuration value
        value: String,
    },
    /// Get a configuration value
    Get {
        /// Configuration key (e.g., "signaling.url")
        key: String,
    },
    /// List all configuration keys
    List,
    /// Show full configuration
    Show {
        /// Output format
        #[arg(long, default_value = "toml")]
        format: OutputFormat,
    },
    /// Unset a configuration value
    Unset {
        /// Configuration key to remove
        key: String,
    },
    /// Test configuration file syntax and validity
    Test,
}

#[derive(Debug, Clone, clap::ValueEnum, Default)]
pub enum OutputFormat {
    /// TOML format (default)
    #[default]
    Toml,
    /// JSON format
    Json,
    /// YAML format
    Yaml,
}

impl ConfigCommand {
    /// Get the configuration file path
    fn config_path(&self) -> &str {
        self.config_file.as_deref().unwrap_or("actr.toml")
    }
}

#[async_trait]
impl Command for ConfigCommand {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        let config_path = self.config_path();

        match &self.command {
            ConfigSubcommand::Set { key, value } => self.set_config(config_path, key, value).await,
            ConfigSubcommand::Get { key } => self.get_config(config_path, key).await,
            ConfigSubcommand::List => self.list_config(config_path).await,
            ConfigSubcommand::Show { format } => self.show_config(config_path, format).await,
            ConfigSubcommand::Unset { key } => self.unset_config(config_path, key).await,
            ConfigSubcommand::Test => self.test_config(config_path).await,
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![] // Config command doesn't require any external components
    }

    fn name(&self) -> &str {
        "config"
    }

    fn description(&self) -> &str {
        "Manage project configuration"
    }
}

impl ConfigCommand {
    /// Set a configuration value
    async fn set_config(&self, config_path: &str, key: &str, value: &str) -> Result<CommandResult> {
        // Load existing config
        if !Path::new(config_path).exists() {
            bail!(
                "Configuration file not found: {}. Run 'actr init' to create a project first.",
                config_path
            );
        }

        let mut raw_config = RawConfig::from_file(config_path)?;

        // Parse the key path and set the value
        self.set_nested_value(&mut raw_config, key, value)?;

        // Save the updated configuration
        raw_config.save_to_file(config_path)?;

        Ok(CommandResult::Success(format!(
            "{} Configuration updated: {} = {}",
            "✅".green(),
            key.cyan(),
            value.yellow()
        )))
    }

    /// Get a configuration value
    async fn get_config(&self, config_path: &str, key: &str) -> Result<CommandResult> {
        if !Path::new(config_path).exists() {
            bail!("Configuration file not found: {}", config_path);
        }

        let raw_config = RawConfig::from_file(config_path)?;

        // Get the nested value
        let value = self.get_nested_value(&raw_config, key)?;

        Ok(CommandResult::Success(value))
    }

    /// List all configuration keys
    async fn list_config(&self, config_path: &str) -> Result<CommandResult> {
        if !Path::new(config_path).exists() {
            return Ok(CommandResult::Success(format!(
                "{} No configuration file found at: {}",
                "📋".yellow(),
                config_path
            )));
        }

        let raw_config = RawConfig::from_file(config_path)?;

        let mut output = String::new();
        output.push_str(&format!("{} Available configuration keys:\n", "📋".cyan()));
        output.push('\n');

        // Package settings
        output.push_str(&format!("  {} Package:\n", "📦".blue()));
        output.push_str("    package.name\n");
        output.push_str("    package.actr_type.manufacturer\n");
        output.push_str("    package.actr_type.name\n");
        if raw_config.package.description.is_some() {
            output.push_str("    package.description\n");
        }

        // System settings
        output.push_str(&format!("\n  {} System:\n", "⚙️".blue()));
        output.push_str("    signaling.url\n");
        output.push_str("    ais_endpoint.url\n");
        output.push_str("    deployment.realm_id\n");
        output.push_str("    discovery.visible\n");
        output.push_str("    storage.mailbox_path\n");

        // WebRTC settings
        output.push_str(&format!("\n  {} WebRTC:\n", "🌐".blue()));
        output.push_str("    webrtc.stun_urls\n");
        output.push_str("    webrtc.turn_urls\n");
        output.push_str("    webrtc.force_relay\n");

        // Observability settings
        output.push_str(&format!("\n  {} Observability:\n", "📊".blue()));
        output.push_str("    observability.filter_level\n");
        output.push_str("    observability.tracing_enabled\n");
        output.push_str("    observability.tracing_endpoint\n");
        output.push_str("    observability.tracing_service_name\n");

        // Exports
        if !raw_config.exports.is_empty() {
            output.push_str(&format!(
                "\n  {} Exports ({} files):\n",
                "📤".blue(),
                raw_config.exports.len()
            ));
            for (i, export) in raw_config.exports.iter().enumerate() {
                output.push_str(&format!("    exports[{}] = {}\n", i, export.display()));
            }
        }

        // Dependencies
        if !raw_config.dependencies.is_empty() {
            output.push_str(&format!(
                "\n  {} Dependencies ({}):\n",
                "🔗".blue(),
                raw_config.dependencies.len()
            ));
            for key in raw_config.dependencies.keys() {
                output.push_str(&format!("    dependencies.{}\n", key));
            }
        }

        // Scripts
        if !raw_config.scripts.is_empty() {
            output.push_str(&format!(
                "\n  {} Scripts ({}):\n",
                "📜".blue(),
                raw_config.scripts.len()
            ));
            for key in raw_config.scripts.keys() {
                output.push_str(&format!("    scripts.{}\n", key));
            }
        }

        Ok(CommandResult::Success(output))
    }

    /// Show full configuration
    async fn show_config(&self, config_path: &str, format: &OutputFormat) -> Result<CommandResult> {
        if !Path::new(config_path).exists() {
            bail!("Configuration file not found: {}", config_path);
        }

        let raw_config = RawConfig::from_file(config_path)?;

        // Output configuration in requested format
        let output = match format {
            OutputFormat::Toml => toml::to_string_pretty(&raw_config)?,
            OutputFormat::Json => serde_json::to_string_pretty(&raw_config)?,
            OutputFormat::Yaml => serde_yaml::to_string(&raw_config)?,
        };

        Ok(CommandResult::Success(output))
    }

    /// Unset a configuration value
    async fn unset_config(&self, config_path: &str, key: &str) -> Result<CommandResult> {
        if !Path::new(config_path).exists() {
            bail!("Configuration file not found: {}", config_path);
        }

        let mut raw_config = RawConfig::from_file(config_path)?;

        // Remove the nested value
        self.unset_nested_value(&mut raw_config, key)?;

        // Save the updated configuration
        raw_config.save_to_file(config_path)?;

        Ok(CommandResult::Success(format!(
            "{} Configuration key '{}' removed successfully",
            "✅".green(),
            key.cyan()
        )))
    }

    /// Test configuration file syntax and validation
    async fn test_config(&self, config_path: &str) -> Result<CommandResult> {
        if !Path::new(config_path).exists() {
            bail!("Configuration file not found: {}", config_path);
        }

        let mut output = String::new();
        output.push_str(&format!(
            "{} Testing configuration file: {}\n\n",
            "🧪".cyan(),
            config_path
        ));

        // Test 1: Raw TOML parsing
        let raw_config = match RawConfig::from_file(config_path) {
            Ok(config) => {
                output.push_str(&format!(
                    "{} Configuration file syntax is valid\n",
                    "✅".green()
                ));
                config
            }
            Err(e) => {
                bail!("Configuration file syntax error:\n   {}", e);
            }
        };

        // Test 2: Full parsing and validation
        match ConfigParser::from_file(config_path) {
            Ok(config) => {
                output.push_str(&format!(
                    "{} Configuration validation passed\n",
                    "✅".green()
                ));

                // Show summary
                output.push_str(&format!("\n{} Configuration Summary:\n", "📋".cyan()));
                output.push_str(&format!("  Package: {}\n", config.package.name.yellow()));
                output.push_str(&format!(
                    "  ActrType: {}\n",
                    config.package.actr_type.to_string_repr().cyan()
                ));

                if let Some(desc) = &config.package.description {
                    output.push_str(&format!("  Description: {}\n", desc));
                }

                output.push_str(&format!("  Realm: {}\n", config.realm.realm_id));
                output.push_str(&format!("  Signaling URL: {}\n", config.signaling_url));
                output.push_str(&format!("  AIS Endpoint: {}\n", config.ais_endpoint.as_deref().unwrap_or("(not set)")));
                output.push_str(&format!(
                    "  Visible in discovery: {}\n",
                    config.visible_in_discovery
                ));

                if !config.dependencies.is_empty() {
                    output.push_str(&format!(
                        "  Dependencies: {} entries\n",
                        config.dependencies.len()
                    ));
                }

                if !config.scripts.is_empty() {
                    output.push_str(&format!("  Scripts: {} entries\n", config.scripts.len()));
                }

                if !raw_config.exports.is_empty() {
                    output.push_str(&format!(
                        "  Exports: {} proto files\n",
                        raw_config.exports.len()
                    ));
                }

                output.push_str(&format!(
                    "\n{} Configuration test completed successfully",
                    "🎯".green()
                ));
            }
            Err(e) => {
                output.push_str(&format!(
                    "{} Configuration validation failed:\n",
                    "❌".red()
                ));
                output.push_str(&format!("   {}\n", e));
                bail!("Configuration validation failed: {}", e);
            }
        }

        Ok(CommandResult::Success(output))
    }

    /// Set a nested configuration value using dot notation
    fn set_nested_value(&self, config: &mut RawConfig, key: &str, value: &str) -> Result<()> {
        let parts: Vec<&str> = key.split('.').collect();

        match parts.as_slice() {
            // Package configuration
            ["package", "name"] => config.package.name = value.to_string(),
            ["package", "description"] => config.package.description = Some(value.to_string()),
            ["package", "actr_type", "manufacturer"] => {
                config.package.actr_type.manufacturer = value.to_string()
            }
            ["package", "actr_type", "name"] => config.package.actr_type.name = value.to_string(),

            // System signaling configuration
            ["signaling", "url"] | ["system", "signaling", "url"] => {
                config.system.signaling.url = Some(value.to_string())
            }
            ["ais_endpoint", "url"] | ["system", "ais_endpoint", "url"] => {
                config.system.ais_endpoint.url = Some(value.to_string())
            }

            // Deployment configuration
            ["deployment", "realm_id"] | ["system", "deployment", "realm_id"] => {
                config.system.deployment.realm_id = Some(
                    value
                        .parse()
                        .map_err(|_| anyhow::anyhow!("deployment.realm_id must be a number"))?,
                );
            }

            // Discovery configuration
            ["discovery", "visible"] | ["system", "discovery", "visible"] => {
                config.system.discovery.visible = Some(
                    value
                        .parse()
                        .map_err(|_| anyhow::anyhow!("discovery.visible must be true or false"))?,
                );
            }

            // Storage configuration
            ["storage", "mailbox_path"] | ["system", "storage", "mailbox_path"] => {
                config.system.storage.mailbox_path = Some(value.into());
            }

            // WebRTC configuration
            ["webrtc", "stun_urls"] | ["system", "webrtc", "stun_urls"] => {
                let urls: Vec<String> = value.split(',').map(|s| s.trim().to_string()).collect();
                config.system.webrtc.stun_urls = urls;
            }
            ["webrtc", "turn_urls"] | ["system", "webrtc", "turn_urls"] => {
                let urls: Vec<String> = value.split(',').map(|s| s.trim().to_string()).collect();
                config.system.webrtc.turn_urls = urls;
            }
            ["webrtc", "force_relay"] | ["system", "webrtc", "force_relay"] => {
                config.system.webrtc.force_relay = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("webrtc.force_relay must be true or false"))?;
            }

            // Observability configuration
            ["observability", "filter_level"] | ["system", "observability", "filter_level"] => {
                config.system.observability.filter_level = Some(value.to_string());
            }
            ["observability", "tracing_enabled"]
            | ["system", "observability", "tracing_enabled"] => {
                config.system.observability.tracing_enabled =
                    Some(value.parse().map_err(|_| {
                        anyhow::anyhow!("observability.tracing_enabled must be true or false")
                    })?);
            }
            ["observability", "tracing_endpoint"]
            | ["system", "observability", "tracing_endpoint"] => {
                config.system.observability.tracing_endpoint = Some(value.to_string());
            }
            ["observability", "tracing_service_name"]
            | ["system", "observability", "tracing_service_name"] => {
                config.system.observability.tracing_service_name = Some(value.to_string());
            }

            // Scripts configuration
            ["scripts", script_name] => {
                config
                    .scripts
                    .insert(script_name.to_string(), value.to_string());
            }

            _ => bail!(
                "Unknown or unsupported configuration key: {}\n\n💡 Hint: Run 'actr config list' to see available keys",
                key
            ),
        }

        Ok(())
    }

    /// Get a nested configuration value using dot notation
    fn get_nested_value(&self, config: &RawConfig, key: &str) -> Result<String> {
        let parts: Vec<&str> = key.split('.').collect();

        let value = match parts.as_slice() {
            // Package configuration
            ["package", "name"] => config.package.name.clone(),
            ["package", "description"] => config.package.description.clone().unwrap_or_default(),
            ["package", "actr_type", "manufacturer"] => {
                config.package.actr_type.manufacturer.clone()
            }
            ["package", "actr_type", "name"] => config.package.actr_type.name.clone(),

            // System signaling configuration
            ["signaling", "url"] | ["system", "signaling", "url"] => {
                config.system.signaling.url.clone().unwrap_or_default()
            }
            ["ais_endpoint", "url"] | ["system", "ais_endpoint", "url"] => {
                config.system.ais_endpoint.url.clone().unwrap_or_default()
            }

            // Deployment configuration
            ["deployment", "realm_id"] | ["system", "deployment", "realm_id"] => config
                .system
                .deployment
                .realm_id
                .map(|r| r.to_string())
                .unwrap_or_default(),

            // Discovery configuration
            ["discovery", "visible"] | ["system", "discovery", "visible"] => config
                .system
                .discovery
                .visible
                .map(|v| v.to_string())
                .unwrap_or_default(),

            // Storage configuration
            ["storage", "mailbox_path"] | ["system", "storage", "mailbox_path"] => config
                .system
                .storage
                .mailbox_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),

            // WebRTC configuration
            ["webrtc", "stun_urls"] | ["system", "webrtc", "stun_urls"] => {
                config.system.webrtc.stun_urls.join(",")
            }
            ["webrtc", "turn_urls"] | ["system", "webrtc", "turn_urls"] => {
                config.system.webrtc.turn_urls.join(",")
            }
            ["webrtc", "force_relay"] | ["system", "webrtc", "force_relay"] => {
                config.system.webrtc.force_relay.to_string()
            }

            // Observability configuration
            ["observability", "filter_level"] | ["system", "observability", "filter_level"] => {
                config
                    .system
                    .observability
                    .filter_level
                    .clone()
                    .unwrap_or_default()
            }
            ["observability", "tracing_enabled"]
            | ["system", "observability", "tracing_enabled"] => config
                .system
                .observability
                .tracing_enabled
                .map(|v| v.to_string())
                .unwrap_or_default(),
            ["observability", "tracing_endpoint"]
            | ["system", "observability", "tracing_endpoint"] => config
                .system
                .observability
                .tracing_endpoint
                .clone()
                .unwrap_or_default(),
            ["observability", "tracing_service_name"]
            | ["system", "observability", "tracing_service_name"] => config
                .system
                .observability
                .tracing_service_name
                .clone()
                .unwrap_or_default(),

            // Scripts configuration
            ["scripts", script_name] => config
                .scripts
                .get(*script_name)
                .cloned()
                .unwrap_or_default(),

            // Dependencies (read-only summary)
            ["dependencies", dep_name] => {
                if let Some(dep) = config.dependencies.get(*dep_name) {
                    match dep {
                        actr_config::RawDependency::Empty {} => "{}".to_string(),
                        actr_config::RawDependency::Specified {
                            actr_type,
                            service,
                            realm,
                        } => {
                            let mut parts = vec![];
                            parts.push(format!("actr_type={}", actr_type));
                            if let Some(s) = service {
                                parts.push(format!("service={}", s));
                            }
                            if let Some(r) = realm {
                                parts.push(format!("realm={}", r));
                            }
                            format!("{{ {} }}", parts.join(", "))
                        }
                    }
                } else {
                    bail!("Dependency not found: {}", dep_name);
                }
            }

            _ => bail!(
                "Unknown configuration key: {}\n\n💡 Hint: Run 'actr config list' to see available keys",
                key
            ),
        };

        Ok(value)
    }

    /// Remove a nested configuration value using dot notation
    fn unset_nested_value(&self, config: &mut RawConfig, key: &str) -> Result<()> {
        let parts: Vec<&str> = key.split('.').collect();

        match parts.as_slice() {
            // Package optional fields
            ["package", "description"] => config.package.description = None,

            // System signaling configuration
            ["signaling", "url"] | ["system", "signaling", "url"] => {
                config.system.signaling.url = None
            }
            ["ais_endpoint", "url"] | ["system", "ais_endpoint", "url"] => {
                config.system.ais_endpoint.url = None
            }

            // Deployment configuration
            ["deployment", "realm_id"] | ["system", "deployment", "realm_id"] => {
                config.system.deployment.realm_id = None
            }

            // Discovery configuration
            ["discovery", "visible"] | ["system", "discovery", "visible"] => {
                config.system.discovery.visible = None
            }

            // Storage configuration
            ["storage", "mailbox_path"] | ["system", "storage", "mailbox_path"] => {
                config.system.storage.mailbox_path = None
            }

            // WebRTC configuration
            ["webrtc", "stun_urls"] | ["system", "webrtc", "stun_urls"] => {
                config.system.webrtc.stun_urls = vec![]
            }
            ["webrtc", "turn_urls"] | ["system", "webrtc", "turn_urls"] => {
                config.system.webrtc.turn_urls = vec![]
            }
            ["webrtc", "force_relay"] | ["system", "webrtc", "force_relay"] => {
                config.system.webrtc.force_relay = false
            }

            // Observability configuration
            ["observability", "filter_level"] | ["system", "observability", "filter_level"] => {
                config.system.observability.filter_level = None
            }
            ["observability", "tracing_enabled"]
            | ["system", "observability", "tracing_enabled"] => {
                config.system.observability.tracing_enabled = None
            }
            ["observability", "tracing_endpoint"]
            | ["system", "observability", "tracing_endpoint"] => {
                config.system.observability.tracing_endpoint = None
            }
            ["observability", "tracing_service_name"]
            | ["system", "observability", "tracing_service_name"] => {
                config.system.observability.tracing_service_name = None
            }

            // Scripts configuration
            ["scripts", script_name] => {
                config.scripts.remove(*script_name);
            }

            // Dependencies configuration
            ["dependencies", dep_name] => {
                config.dependencies.remove(*dep_name);
            }

            // Cannot unset required fields
            ["package", "name"]
            | ["package", "actr_type", "manufacturer"]
            | ["package", "actr_type", "name"] => {
                bail!("Cannot unset required configuration key: {}", key);
            }

            _ => bail!("Cannot unset configuration key: {}", key),
        }

        Ok(())
    }
}
