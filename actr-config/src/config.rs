//! Main configuration structures for actr.toml

use crate::dependencies::ProtoDependencies;
use crate::error::{ActrConfigError, Result};
use crate::routing::RoutingConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Complete actr.toml configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActrConfig {
    /// Package information
    pub package: PackageConfig,

    /// Dependencies configuration
    #[serde(default)]
    pub dependencies: DependenciesConfig,

    /// Routing rules
    #[serde(default)]
    pub routing: RoutingConfig,
}

/// Package metadata configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageConfig {
    /// Package name
    pub name: String,

    /// Package version
    pub version: String,

    /// Rust edition
    #[serde(default = "default_edition")]
    pub edition: String,

    /// Package description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Package authors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,

    /// Package license
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

fn default_edition() -> String {
    "2021".to_string()
}

/// Dependencies configuration container
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DependenciesConfig {
    /// Proto dependencies
    #[serde(default)]
    pub protos: ProtoDependencies,
}

impl ActrConfig {
    /// Load configuration from a file path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_str(&content)
    }

    /// Parse configuration from a string
    pub fn from_str(content: &str) -> Result<Self> {
        let config: ActrConfig = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// Save configuration to a file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| {
            ActrConfigError::ValidationError(format!("Failed to serialize config: {}", e))
        })?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate package configuration
        self.package.validate()?;

        // Validate dependencies
        for (name, dep) in &self.dependencies.protos.dependencies {
            dep.validate().map_err(|e| {
                ActrConfigError::ValidationError(format!(
                    "Invalid proto dependency '{}': {}",
                    name, e
                ))
            })?;
        }

        // Validate routing rules
        self.routing.validate()?;

        Ok(())
    }

    /// Get a default configuration template
    pub fn default_template(package_name: impl Into<String>) -> Self {
        Self {
            package: PackageConfig {
                name: package_name.into(),
                version: "0.1.0".to_string(),
                edition: "2021".to_string(),
                description: None,
                authors: None,
                license: None,
            },
            dependencies: DependenciesConfig::default(),
            routing: RoutingConfig::default(),
        }
    }

    /// Check if this project uses auto-runner mode (no main.rs)
    pub fn is_auto_runner_mode(&self) -> bool {
        // In auto-runner mode, there should be no src/main.rs file
        !Path::new("src/main.rs").exists()
    }
}

impl PackageConfig {
    /// Validate the package configuration
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(ActrConfigError::MissingField("package.name".to_string()));
        }

        if self.version.is_empty() {
            return Err(ActrConfigError::MissingField("package.version".to_string()));
        }

        // Validate package name format (basic validation)
        if !self
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ActrConfigError::ValidationError(
                "Package name can only contain alphanumeric characters, hyphens, and underscores"
                    .to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependencies::ProtoDependency;
    use crate::routing::RoutingRule;

    #[test]
    fn test_basic_config_parsing() {
        let toml_content = r#"
[package]
name = "my-actor"
version = "0.1.0"
edition = "2021"

[dependencies.protos]
user_service = { git = "https://github.com/org/protos.git", path = "user.proto", tag = "v1.0.0" }

[routing]
"user.v1.GetUserRequest" = { call = "user.v1.UserService" }
"#;

        let config = ActrConfig::from_str(toml_content).unwrap();
        assert_eq!(config.package.name, "my-actor");
        assert_eq!(config.package.version, "0.1.0");
        assert!(config
            .dependencies
            .protos
            .dependencies
            .contains_key("user_service"));
        assert!(config.routing.rules.contains_key("user.v1.GetUserRequest"));
    }

    #[test]
    fn test_default_template() {
        let config = ActrConfig::default_template("test-actor");
        assert_eq!(config.package.name, "test-actor");
        assert_eq!(config.package.version, "0.1.0");
        assert_eq!(config.package.edition, "2021");
    }

    #[test]
    fn test_config_validation() {
        let mut config = ActrConfig::default_template("test-actor");
        assert!(config.validate().is_ok());

        // Test invalid package name
        config.package.name = "".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_package_name_validation() {
        let mut config = PackageConfig {
            name: "valid-name_123".to_string(),
            version: "0.1.0".to_string(),
            edition: "2021".to_string(),
            description: None,
            authors: None,
            license: None,
        };
        assert!(config.validate().is_ok());

        config.name = "invalid name!".to_string();
        assert!(config.validate().is_err());
    }
}
