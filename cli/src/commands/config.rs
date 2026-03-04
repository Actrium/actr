//! Config command implementation - manage project configuration

use crate::commands::Command;
use crate::error::{ActrCliError, Result};
use actr::config::{Config, ConfigParser};
use async_trait::async_trait;
use clap::{Args, Subcommand};
use std::path::Path;
use tracing::info;

#[derive(Args)]
pub struct ConfigCommand {
    /// Configuration file to load (defaults to Actr.toml)
    #[arg(short = 'f', long = "file")]
    pub config_file: Option<String>,

    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Subcommand)]
pub enum ConfigSubcommand {
    /// Set a configuration value
    Set {
        /// Configuration key (e.g., "build.output-dir")
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
    /// Test configuration file syntax
    Test,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    /// TOML format (default)
    Toml,
    /// JSON format
    Json,
    /// YAML format
    Yaml,
}

#[async_trait]
impl Command for ConfigCommand {
    async fn execute(&self) -> Result<()> {
        let config_path = self.config_file.as_deref().unwrap_or("Actr.toml");
        
        match &self.command {
            ConfigSubcommand::Set { key, value } => self.set_config(config_path, key, value).await,
            ConfigSubcommand::Get { key } => self.get_config(config_path, key).await,
            ConfigSubcommand::List => self.list_config(config_path).await,
            ConfigSubcommand::Show { format } => self.show_config(config_path, format).await,
            ConfigSubcommand::Unset { key } => self.unset_config(config_path, key).await,
            ConfigSubcommand::Test => self.test_config(config_path).await,
        }
    }
}

impl ConfigCommand {
    /// Set a configuration value
    async fn set_config(&self, config_path: &str, key: &str, value: &str) -> Result<()> {
        info!("⚙️ Setting configuration: {} = {}", key, value);

        let mut config = if Path::new(config_path).exists() {
            ConfigParser::from_file(config_path)?
        } else {
            Config::default_template("default-project", None)
        };

        // Parse the key path and set the value
        self.set_nested_value(&mut config, key, value)?;

        // Save the updated configuration
        config.save_to_file(config_path)?;
        
        info!("✅ Configuration updated successfully");
        Ok(())
    }

    /// Get a configuration value
    async fn get_config(&self, config_path: &str, key: &str) -> Result<()> {
        if !Path::new(config_path).exists() {
            return Err(ActrCliError::config_error(format!(
                "Configuration file not found: {}",
                config_path
            )));
        }

        let config = ConfigParser::from_file(config_path)?;
        
        // Get the nested value
        let value = self.get_nested_value(&config, key)?;
        println!("{}", value);
        
        Ok(())
    }

    /// List all configuration keys
    async fn list_config(&self, config_path: &str) -> Result<()> {
        if !Path::new(config_path).exists() {
            println!("📋 No configuration file found");
            return Ok(());
        }

        let config = ConfigParser::from_file(config_path)?;
        
        println!("📋 Available configuration keys:");
        
        // List package settings
        println!("  package.name");
        println!("  package.version");
        if config.package.description.is_some() {
            println!("  package.description");
        }
        
        // List system settings
        if let Some(_) = &config.system.signaling.url {
            println!("  system.signaling.url");
        }
        
        // List build settings
        println!("  build.output-dir");
        println!("  build.clean");
        println!("  build.release");
        println!("  build.verbose");
        println!("  build.target");
        println!("  build.features");
        
        // List provides
        if !config.provides.proto.is_empty() {
            println!("  Provides:");
            for key in config.provides.proto.keys() {
                println!("    provides.{}", key);
            }
        }
        
        // List dependencies
        if !config.dependencies.is_empty() {
            println!("  Dependencies:");
            for key in config.dependencies.keys() {
                println!("    dependencies.{}", key);
            }
        }
        
        Ok(())
    }

    /// Show full configuration
    async fn show_config(&self, config_path: &str, format: &OutputFormat) -> Result<()> {
        if !Path::new(config_path).exists() {
            return Err(ActrCliError::config_error(format!(
                "Configuration file not found: {}",
                config_path
            )));
        }

        let config = ConfigParser::from_file(config_path)?;

        // Output configuration in requested format
        match format {
            OutputFormat::Toml => {
                let output = toml::to_string_pretty(&config).map_err(|e| {
                    ActrCliError::config_error(format!(
                        "Failed to serialize configuration to TOML: {}",
                        e
                    ))
                })?;
                println!("{}", output);
            }
            OutputFormat::Json => {
                let output = serde_json::to_string_pretty(&config).map_err(|e| {
                    ActrCliError::config_error(format!(
                        "Failed to serialize configuration to JSON: {}",
                        e
                    ))
                })?;
                println!("{}", output);
            }
            OutputFormat::Yaml => {
                let output = serde_yaml::to_string(&config).map_err(|e| {
                    ActrCliError::config_error(format!(
                        "Failed to serialize configuration to YAML: {}",
                        e
                    ))
                })?;
                println!("{}", output);
            }
        }

        Ok(())
    }

    /// Unset a configuration value
    async fn unset_config(&self, config_path: &str, key: &str) -> Result<()> {
        if !Path::new(config_path).exists() {
            return Err(ActrCliError::config_error(format!(
                "Configuration file not found: {}",
                config_path
            )));
        }

        let mut config = ConfigParser::from_file(config_path)?;
        
        // Remove the nested value
        self.unset_nested_value(&mut config, key)?;
        
        // Save the updated configuration
        config.save_to_file(config_path)?;
        
        info!("✅ Configuration key '{}' removed successfully", key);
        Ok(())
    }

    /// Set a nested configuration value using dot notation
    fn set_nested_value(&self, config: &mut Config, key: &str, value: &str) -> Result<()> {
        let parts: Vec<&str> = key.split('.').collect();
        
        match parts.as_slice() {
            ["package", "name"] => config.package.name = value.to_string(),
            ["package", "version"] => config.package.version = value.to_string(),
            ["package", "description"] => config.package.description = Some(value.to_string()),
            ["package", "type"] => config.package.r#type = Some(value.to_string()),
            ["system", "signaling", "url"] => config.system.signaling.url = Some(value.to_string()),
            // Build configuration
            ["build", "output-dir"] => config.build.output_dir = Some(value.to_string()),
            ["build", "clean"] => config.build.clean = Some(value.parse().map_err(|_| 
                ActrCliError::config_error("build.clean must be true or false".to_string()))?),
            ["build", "release"] => config.build.release = Some(value.parse().map_err(|_| 
                ActrCliError::config_error("build.release must be true or false".to_string()))?),
            ["build", "verbose"] => config.build.verbose = Some(value.parse().map_err(|_| 
                ActrCliError::config_error("build.verbose must be true or false".to_string()))?),
            ["build", "target"] => config.build.target = Some(value.to_string()),
            ["build", "features"] => {
                let features: Vec<String> = value.split(',').map(|s| s.trim().to_string()).collect();
                config.build.features = Some(features);
            },
            // Provides configuration (without proto segment)
            ["provides", proto_name] => {
                config.provides.proto.insert(proto_name.to_string(), value.to_string());
            },
            // Dependencies configuration (without proto segment)
            ["dependencies", _dep_name] => {
                // TODO: Dependencies are Vec<Dependency> in new API, not HashMap
                // Direct modification not supported - use TOML editing instead
                return Err(ActrCliError::config_error(
                    "Direct dependency modification not yet supported. Edit Actr.toml manually.".to_string()
                ));
            },
            _ => return Err(ActrCliError::config_error(format!(
                "Unknown configuration key: {}",
                key
            ))),
        }
        
        Ok(())
    }

    /// Get a nested configuration value using dot notation
    fn get_nested_value(&self, config: &Config, key: &str) -> Result<String> {
        let parts: Vec<&str> = key.split('.').collect();
        
        let value = match parts.as_slice() {
            ["package", "name"] => config.package.name.clone(),
            ["package", "version"] => config.package.version.clone(),
            ["package", "description"] => config.package.description.clone().unwrap_or_default(),
            ["package", "type"] => config.package.r#type.clone().unwrap_or_default(),
            ["system", "signaling", "url"] => config.system.signaling.url.clone().unwrap_or_default(),
            // Build configuration
            ["build", "output-dir"] => config.build.output_dir.clone().unwrap_or_default(),
            ["build", "clean"] => config.build.clean.map(|b| b.to_string()).unwrap_or_default(),
            ["build", "release"] => config.build.release.map(|b| b.to_string()).unwrap_or_default(),
            ["build", "verbose"] => config.build.verbose.map(|b| b.to_string()).unwrap_or_default(),
            ["build", "target"] => config.build.target.clone().unwrap_or_default(),
            ["build", "features"] => {
                config.build.features.clone()
                    .map(|features| features.join(","))
                    .unwrap_or_default()
            },
            // Provides configuration (without proto segment)
            ["provides", proto_name] => {
                config.provides.proto.get(*proto_name)
                    .cloned()
                    .unwrap_or_default()
            },
            // Dependencies configuration (without proto segment)
            ["dependencies", dep_name] => {
                // Dependencies are now Vec<Dependency>, find by alias
                if let Some(dep) = config.dependencies.iter().find(|d| d.alias == *dep_name) {
                    // Return ActrType as string
                    format!("{}:{}", dep.actr_type.manufacturer, dep.actr_type.name)
                } else {
                    return Err(ActrCliError::config_error(format!(
                        "Dependency not found: {}",
                        dep_name
                    )));
                }
            },
            _ => return Err(ActrCliError::config_error(format!(
                "Unknown configuration key: {}",
                key
            ))),
        };
        
        Ok(value)
    }

    /// Remove a nested configuration value using dot notation
    fn unset_nested_value(&self, config: &mut Config, key: &str) -> Result<()> {
        let parts: Vec<&str> = key.split('.').collect();
        
        match parts.as_slice() {
            ["package", "description"] => config.package.description = None,
            ["package", "type"] => config.package.r#type = None,
            ["system", "signaling", "url"] => config.system.signaling.url = None,
            // Build configuration
            ["build", "output-dir"] => config.build.output_dir = None,
            ["build", "clean"] => config.build.clean = None,
            ["build", "release"] => config.build.release = None,
            ["build", "verbose"] => config.build.verbose = None,
            ["build", "target"] => config.build.target = None,
            ["build", "features"] => config.build.features = None,
            // Provides configuration (without proto segment)
            ["provides", proto_name] => {
                config.provides.proto.remove(*proto_name);
            },
            // Dependencies configuration (without proto segment)
            ["dependencies", dep_name] => {
                config.dependencies.remove(*dep_name);
            },
            _ => return Err(ActrCliError::config_error(format!(
                "Cannot unset configuration key: {}",
                key
            ))),
        }
        
        Ok(())
    }

    /// Test configuration file syntax and validation
    async fn test_config(&self, config_path: &str) -> Result<()> {
        info!("🧪 Testing configuration file: {}", config_path);

        if !Path::new(config_path).exists() {
            return Err(ActrCliError::config_error(format!(
                "Configuration file not found: {}",
                config_path
            )));
        }

        // Test parsing
        match ConfigParser::from_file(config_path) {
            Ok(config) => {
                println!("✅ Configuration file syntax is valid");
                
                // Test validation
                match config.validate() {
                    Ok(()) => {
                        println!("✅ Configuration validation passed");
                        
                        // Show summary
                        println!("\n📋 Configuration Summary:");
                        println!("  Package: {} v{}", config.package.name, config.package.version);
                        
                        if let Some(service_type) = &config.package.r#type {
                            println!("  Service Type: {}", service_type);
                        }
                        
                        if !config.dependencies.is_empty() {
                            println!("  Dependencies: {} entries", config.dependencies.len());
                        }
                        
                        if !config.scripts.scripts.is_empty() {
                            println!("  Scripts: {} entries", config.scripts.scripts.len());
                        }
                        
                        println!("\n🎯 Configuration test completed successfully");
                    }
                    Err(validation_error) => {
                        println!("❌ Configuration validation failed:");
                        println!("   {}", validation_error);
                        return Err(ActrCliError::config_error(format!(
                            "Configuration validation failed: {}",
                            validation_error
                        )));
                    }
                }
            }
            Err(parse_error) => {
                println!("❌ Configuration file syntax error:");
                println!("   {}", parse_error);
                return Err(ActrCliError::config_error(format!(
                    "Failed to parse configuration file: {}",
                    parse_error
                )));
            }
        }

        Ok(())
    }
}