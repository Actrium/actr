//! Check command implementation - verify Actor-RTC service availability
//!
//! The check command validates that services are available in the registry
//! and optionally verifies they match the configured dependencies.

use crate::core::{Command, CommandContext, CommandResult, ComponentType, NetworkCheckOptions};
use actr_config::ConfigParser;
use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::Args;
use comfy_table::{Attribute, Cell, Color, Table};
use futures_util::future;
use owo_colors::OwoColorize;
use tracing::info;

/// Check command - validates service availability
#[derive(Args, Debug)]
#[command(
    about = "Validate project dependencies",
    long_about = "Validate that services are available in the registry and match the configured dependencies"
)]
pub struct CheckCommand {
    /// Service names to check (e.g., "user-service", "order-service")
    /// If not provided, checks all services from the configuration file
    #[arg(value_name = "SERVICE_NAME")]
    pub packages: Vec<String>,

    /// Manifest file to load services from (defaults to manifest.toml)
    #[arg(short = 'm', long = "manifest-path")]
    pub manifest_path: Option<String>,

    /// Show detailed connection information
    #[arg(short, long)]
    pub verbose: bool,

    /// Timeout for each service check in seconds
    #[arg(long, default_value = "10")]
    pub timeout: u64,

    /// Also verify services are installed in manifest.lock.toml
    #[arg(long)]
    pub lock: bool,
}

#[async_trait]
impl Command for CheckCommand {
    async fn execute(&self, context: &CommandContext) -> Result<CommandResult> {
        let config_path = self.manifest_path.as_deref().unwrap_or("manifest.toml");

        let pipeline = {
            let mut container = context.container.lock().unwrap();
            container.get_validation_pipeline()?
        };
        let options = NetworkCheckOptions::with_timeout_secs(self.timeout);

        println!("🔍 Starting dependency validation...");
        info!("🔍 Starting dependency validation...");

        // 1. Validate Config and Signaling Server
        let config_validation = pipeline.config_manager().validate_config().await?;
        if !config_validation.is_valid {
            let mut msg = format!("{} Configuration validation failed:\n", "❌".red());
            for err in config_validation.errors {
                msg.push_str(&format!("  - {}\n", err.red()));
            }
            return Ok(CommandResult::Error(msg));
        }

        let config = ConfigParser::from_manifest_file(config_path)
            .with_context(|| format!("Failed to load manifest: {}", config_path))?;

        // Resolve signaling URL from default + global CLI config + project CLI config.
        // `resolve_effective_cli_config` applies built-in defaults, so we always have a URL.
        let cli_config =
            crate::config::resolver::resolve_effective_cli_config().unwrap_or_default();
        let signaling_url = cli_config.network.signaling_url;

        println!("🌐 Checking signaling server: {}...", signaling_url);
        info!("🌐 Checking signaling server: {}...", signaling_url);
        let signaling_status = pipeline
            .network_validator()
            .check_connectivity(&signaling_url, &options)
            .await?;
        if signaling_status.is_reachable {
            let latency = signaling_status.response_time_ms.unwrap_or(0);
            println!("  ✔ Signaling server is reachable ({}ms)", latency);
            info!("  ✔ Signaling server is reachable ({}ms)", latency);
        } else {
            let err = signaling_status
                .error
                .unwrap_or_else(|| "Unknown error".to_string());
            return Ok(CommandResult::Error(format!(
                "{} Signaling server unreachable: {}",
                "❌".red(),
                err.red()
            )));
        }

        // 2. Resolve Dependencies to check

        let all_specs = pipeline.dependency_resolver().resolve_spec(&config).await?;

        let specs_to_check = if self.packages.is_empty() {
            all_specs
        } else {
            all_specs
                .into_iter()
                .filter(|s| self.packages.contains(&s.name) || self.packages.contains(&s.alias))
                .collect()
        };

        if specs_to_check.is_empty() {
            if self.packages.is_empty() {
                return Ok(CommandResult::Success(
                    "No dependencies to check".to_string(),
                ));
            } else {
                return Ok(CommandResult::Error(format!(
                    "None of the specified packages found in {}",
                    config_path
                )));
            }
        }

        // 3. Perform Validation via Pipeline
        let dep_validations = pipeline.validate_dependencies(&specs_to_check).await?;

        // 3.1 Lock File Validation (if requested)
        if self.lock {
            println!("🔒 Verifying lock file integrity...");
            info!("🔒 Verifying lock file integrity...");
            let lock_path = std::path::Path::new("manifest.lock.toml");
            if !lock_path.exists() {
                return Ok(CommandResult::Error(
                    "manifest.lock.toml not found".to_string(),
                ));
            }

            let lock_file = actr_config::LockFile::from_file(lock_path)
                .map_err(|e| anyhow::anyhow!("Failed to read lock file: {}", e))?;

            for spec in &specs_to_check {
                if let Some(locked) = lock_file.get_dependency(&spec.name) {
                    // Check if versions/types match if necessary
                    if let Some(spec_fp) = &spec.fingerprint
                        && spec_fp != &locked.fingerprint
                    {
                        return Ok(CommandResult::Error(format!(
                            "{} Fingerprint mismatch for '{}' in lock file:\n  Expected: {}\n  Locked:   {}",
                            "❌".red(),
                            spec.alias,
                            spec_fp,
                            locked.fingerprint
                        )));
                    }
                } else {
                    return Ok(CommandResult::Error(format!(
                        "{} Dependency '{}' not found in manifest.lock.toml",
                        "❌".red(),
                        spec.alias
                    )));
                }
            }
            println!("  ✔ Lock file integrity verified");
            info!("  ✔ Lock file integrity verified");
        }

        // For network and fingerprint, we need ResolvedDependency
        // Use parallel fetch for service details to improve performance
        let fetch_futures = specs_to_check.iter().map(|spec| {
            let sd = pipeline.service_discovery().clone();
            let name = spec.name.clone();
            async move {
                let details = sd.get_service_details(&name).await;
                (name, details)
            }
        });

        let fetch_results = future::join_all(fetch_futures).await;

        let mut service_details_map = std::collections::HashMap::new();
        let mut fetch_errors: Vec<(String, anyhow::Error)> = Vec::new();

        for (name, result) in fetch_results {
            match result {
                Ok(details) => {
                    service_details_map.insert(name, details);
                }
                Err(e) => {
                    fetch_errors.push((name, e));
                }
            }
        }

        let resolved_deps: Vec<_> = specs_to_check
            .iter()
            .map(|spec| {
                let details = service_details_map.get(&spec.name);
                crate::core::ResolvedDependency {
                    spec: spec.clone(),
                    fingerprint: details
                        .map(|d| d.info.fingerprint.clone())
                        .unwrap_or_default(),
                    proto_files: details.map(|d| d.proto_files.clone()).unwrap_or_default(),
                }
            })
            .collect();

        let net_validations = pipeline
            .validate_network_connectivity(&resolved_deps, &options)
            .await?;
        let fp_validations = pipeline.validate_fingerprints(&resolved_deps).await?;

        // 4. Report Results
        let mut table = Table::new();
        table.set_header(vec![
            Cell::new("Dependency").add_attribute(Attribute::Bold),
            Cell::new("Availability").add_attribute(Attribute::Bold),
            Cell::new("Network").add_attribute(Attribute::Bold),
            Cell::new("Fingerprint").add_attribute(Attribute::Bold),
        ]);

        let mut all_ok = true;

        for i in 0..specs_to_check.len() {
            let spec = &specs_to_check[i];
            let dep_v = &dep_validations[i];
            let net_v = &net_validations[i];
            let fp_v = &fp_validations[i];

            let mut row = vec![Cell::new(&spec.alias)];

            // Availability
            if dep_v.is_available {
                row.push(Cell::new("✔ Available").fg(Color::Green));
            } else {
                let err_msg = if self.verbose {
                    dep_v.error.as_deref().unwrap_or("Unknown error")
                } else {
                    "Missing"
                };
                row.push(Cell::new(format!("✘ {}", err_msg)).fg(Color::Red));
                all_ok = false;
            }

            // Network
            if !net_v.is_applicable {
                let cell_text = if self.verbose {
                    net_v.error.clone().unwrap_or_else(|| "N/A".to_string())
                } else {
                    "N/A".to_string()
                };
                row.push(Cell::new(cell_text).fg(Color::Yellow));
            } else if net_v.is_reachable {
                let latency = net_v
                    .latency_ms
                    .map(|l| format!(" ({}ms)", l))
                    .unwrap_or_default();
                row.push(Cell::new(format!("✔ Reachable{}", latency)).fg(Color::Green));
            } else {
                // If service detail fetch failed earlier, network check will likely fail too.
                // Check if we had a fetch error for this service
                let fetch_err = fetch_errors
                    .iter()
                    .find(|(n, _)| n == &spec.name)
                    .map(|(_, e)| e.to_string());

                let err_display = if self.verbose {
                    if let Some(fe) = fetch_err {
                        format!("Fetch Error: {}", fe)
                    } else {
                        net_v
                            .error
                            .clone()
                            .unwrap_or_else(|| "Unreachable".to_string())
                    }
                } else {
                    "✘ Unreachable".to_string()
                };

                row.push(Cell::new(err_display).fg(Color::Red));
                all_ok = false;
            }

            // Fingerprint
            if fp_v.is_valid {
                row.push(Cell::new("✔ Match").fg(Color::Green));
            } else {
                let mut cell_text = "✘ Mismatch".to_string();
                if self.verbose {
                    let expected = &fp_v.expected.value;
                    let actual_opt = fp_v.actual.as_ref().map(|f| &f.value);

                    if let Some(actual) = actual_opt {
                        if !expected.is_empty() {
                            cell_text = format!(
                                "✘ Mismatch\n  Exp: {:.8}...\n  Act: {:.8}...",
                                expected, actual
                            );
                        }
                    } else if let Some(err) = &fp_v.error {
                        cell_text = format!("✘ Error: {}", err);
                    }
                }
                row.push(Cell::new(cell_text).fg(Color::Red));
                all_ok = false;
            }

            table.add_row(row);
        }

        println!("\n{table}");

        if all_ok {
            Ok(CommandResult::Success(format!(
                "\n{} All {} services passed validation!",
                "✨".green(),
                specs_to_check.len()
            )))
        } else {
            Ok(CommandResult::Error(format!(
                "\n{} Some services failed validation. Run with --verbose for details.",
                "⚠️".yellow()
            )))
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![
            ComponentType::ConfigManager,
            ComponentType::DependencyResolver,
            ComponentType::ServiceDiscovery,
            ComponentType::NetworkValidator,
            ComponentType::FingerprintValidator,
        ]
    }

    fn name(&self) -> &str {
        "check"
    }

    fn description(&self) -> &str {
        "Validate project dependencies and service availability"
    }
}
