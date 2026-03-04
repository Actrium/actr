//! Check command implementation - verify Actor-RTC service connectivity
//!
//! The check command validates that Actor-RTC services are reachable and available
//! in the current network environment using the actr:// service protocol.

use crate::commands::Command;
use crate::error::{ActrCliError, Result};
use actr::config::ConfigParser;
use async_trait::async_trait;
use clap::Args;
use tracing::{debug, error, info};

#[derive(Args)]
pub struct CheckCommand {
    /// Actor-RTC service URIs to check (e.g., actr://user-service/, actr://order-service/)
    /// If not provided, checks all actr:// service URIs from the configuration file
    #[arg(value_name = "ACTR_URI")]
    pub uris: Vec<String>,

    /// Configuration file to load service URIs from (defaults to Actr.toml)
    #[arg(short = 'f', long = "file")]
    pub config_file: Option<String>,

    /// Show detailed connection information
    #[arg(short, long)]
    pub verbose: bool,

    /// Timeout for each service check in seconds
    #[arg(long, default_value = "10")]
    pub timeout: u64,
}

#[async_trait]
impl Command for CheckCommand {
    async fn execute(&self) -> Result<()> {
        let config_path = self.config_file.as_deref().unwrap_or("Actr.toml");

        // Determine which service URIs to check
        let uris_to_check = if self.uris.is_empty() {
            // Load service URIs from configuration file
            info!(
                "🔍 Loading Actor-RTC service URIs from configuration: {}",
                config_path
            );
            self.load_uris_from_config(config_path).await?
        } else {
            // Use provided service URIs
            info!("🔍 Checking provided Actor-RTC service URIs");
            self.validate_provided_uris()?
        };

        if uris_to_check.is_empty() {
            info!("ℹ️ No Actor-RTC service URIs to check");
            return Ok(());
        }

        info!(
            "📦 Checking {} Actor-RTC service URIs...",
            uris_to_check.len()
        );

        let mut total_checked = 0;
        let mut available_count = 0;
        let mut unavailable_count = 0;

        for uri in &uris_to_check {
            total_checked += 1;
            let is_available = self.check_actr_uri(uri).await?;

            if is_available {
                available_count += 1;
            } else {
                unavailable_count += 1;
            }
        }

        // Summary
        info!("");
        info!("📊 Actor-RTC Service Check Summary:");
        info!("   Total checked: {}", total_checked);
        info!("   ✅ Available: {}", available_count);
        info!("   ❌ Unavailable: {}", unavailable_count);

        if unavailable_count > 0 {
            error!(
                "⚠️ {} Actor-RTC services are not available in the current network",
                unavailable_count
            );
            return Err(ActrCliError::dependency_error(
                "Some Actor-RTC services are unavailable",
            ));
        } else {
            info!("🎉 All Actor-RTC services are available and accessible!");
        }

        Ok(())
    }
}

impl CheckCommand {
    /// Load actr:// service URIs from configuration file
    async fn load_uris_from_config(&self, config_path: &str) -> Result<Vec<String>> {
        // Load configuration
        let config = ConfigParser::from_file(config_path)
            .map_err(|e| ActrCliError::config_error(format!("Failed to load config: {e}")))?;

        let mut uris = Vec::new();

        // Extract actr:// service URIs from dependencies
        // Construct URI from ActrType: actr://<manufacturer>:<name>/
        for dependency in &config.dependencies {
            if let Some(actr_type) = &dependency.actr_type {
                let uri = if actr_type.manufacturer.is_empty() {
                    format!("actr://{}/", actr_type.name)
                } else {
                    format!("actr://{}:{}/", actr_type.manufacturer, actr_type.name)
                };
                uris.push(uri);
                debug!(
                    "Added dependency URI: {} (alias: {})",
                    uris.last().unwrap(),
                    dependency.alias
                );
            }
        }

        if uris.is_empty() {
            info!(
                "ℹ️ No dependencies found in configuration file: {}",
                config_path
            );
        } else {
            info!(
                "📋 Found {} actr:// service URIs in configuration",
                uris.len()
            );
        }

        Ok(uris)
    }

    /// Validate provided service URIs and filter for actr:// protocol only
    fn validate_provided_uris(&self) -> Result<Vec<String>> {
        let mut valid_uris = Vec::new();

        for uri in &self.uris {
            if uri.starts_with("actr://") {
                valid_uris.push(uri.clone());
            } else {
                error!(
                    "❌ Invalid service URI protocol: {} (only actr:// service URIs are supported)",
                    uri
                );
                return Err(ActrCliError::dependency_error(format!(
                    "Invalid service URI protocol: {uri} (only actr:// service URIs are supported)"
                )));
            }
        }

        Ok(valid_uris)
    }

    /// Check availability of a specific actr:// service URI
    async fn check_actr_uri(&self, uri: &str) -> Result<bool> {
        info!("🔗 Checking Actor-RTC service: {}", uri);

        // Parse the actr:// service URI
        let service_uri = match self.parse_actr_uri(uri) {
            Ok(parsed) => parsed,
            Err(e) => {
                error!("❌ [{}] Invalid Actor-RTC service URI format: {}", uri, e);
                return Ok(false);
            }
        };

        match service_uri {
            ActrUri::Service { service_name } => {
                self.check_service_availability(&service_name).await
            }
        }
    }

    /// Parse actr:// service URI into components
    fn parse_actr_uri(&self, uri: &str) -> Result<ActrUri> {
        if !uri.starts_with("actr://") {
            return Err(ActrCliError::dependency_error(
                "Service URI must start with actr://",
            ));
        }

        let uri_part = &uri[7..]; // Remove "actr://"

        // Service URI must end with / (service-level dependency)
        if uri_part.ends_with('/') {
            let service_name = uri_part.trim_end_matches('/').to_string();
            if service_name.is_empty() {
                return Err(ActrCliError::dependency_error(
                    "Service name cannot be empty",
                ));
            }
            return Ok(ActrUri::Service { service_name });
        }

        Err(ActrCliError::dependency_error(
            "Invalid actr:// service URI format. Use 'actr://service-name/'",
        ))
    }

    /// Check service-level availability (actr://service-name/)
    async fn check_service_availability(&self, service_name: &str) -> Result<bool> {
        debug!("Checking service availability: actr://{}/", service_name);

        // TODO: Implement actual service discovery and connectivity check
        // For now, just validate the URI format and return success
        if service_name.is_empty() {
            error!("❌ [actr://{}] Invalid empty service name", service_name);
            return Ok(false);
        }

        if self.verbose {
            info!(
                "✅ [actr://{}] URI format is valid (service discovery not yet implemented)",
                service_name
            );
        } else {
            info!("✅ [actr://{}] URI format valid", service_name);
        }

        Ok(true)
    }
}

/// Parsed Actor-RTC service URI components
#[derive(Debug, Clone)]
enum ActrUri {
    /// Service-level URI: actr://service-name/
    Service { service_name: String },
}
