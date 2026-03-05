//! Discovery Command Implementation
//!
//! Demonstrates multi-level reuse patterns: Service Discovery -> Validation -> Optional Install

use anyhow::Result;
use async_trait::async_trait;
use clap::Args;

use crate::core::{
    ActrCliError, Command, CommandContext, CommandResult, ComponentType, ConfigManager,
    DependencyResolver, DependencySpec, Fingerprint, FingerprintValidator, NetworkCheckOptions,
    NetworkValidator, ResolvedDependency, ServiceDetails, ServiceDiscovery, ServiceInfo,
};

/// Discovery 命令
#[derive(Args, Debug)]
#[command(
    about = "Discover network services",
    long_about = "Discover Actor services in the network, view available services and choose to install"
)]
pub struct DiscoveryCommand {
    /// Service name filter pattern (e.g., user-*)
    #[arg(long, value_name = "PATTERN")]
    pub filter: Option<String>,

    /// Show detailed information
    #[arg(long)]
    pub verbose: bool,

    /// Automatically install selected services
    #[arg(long)]
    pub auto_install: bool,
}

#[async_trait]
impl Command for DiscoveryCommand {
    async fn execute(&self, context: &CommandContext) -> Result<CommandResult> {
        // Get reusable components
        let (service_discovery, user_interface, config_manager) = {
            let container = context.container.lock().unwrap();
            (
                container.get_service_discovery()?,
                container.get_user_interface()?,
                container.get_config_manager()?,
            )
        };

        // Phase 1: Service Discovery

        let filter = self.create_service_filter();
        let services = service_discovery.discover_services(filter.as_ref()).await?;
        tracing::debug!("Discovered services: {:?}", services);

        if services.is_empty() {
            println!("ℹ️ No available Actor services discovered in the current network");
            return Ok(CommandResult::Success("No services discovered".to_string()));
        }

        println!("🔍 Discovered Actor services:");
        // Display discovered services table
        self.display_services_table(&services);

        // Selection Phase
        let service_options: Vec<String> = services.iter().map(|s| s.name.clone()).collect();

        let selected_index = match user_interface
            .select_from_list(&service_options, "Select a service to view (Esc to quit)")
            .await
        {
            Ok(index) => index,
            Err(err) if Self::is_operation_cancelled(&err) => {
                return Ok(CommandResult::Success("Operation cancelled".to_string()));
            }
            Err(err) => return Err(err),
        };

        let selected_service = &services[selected_index];
        let mut selected_details = None;

        if self.verbose {
            let details = service_discovery
                .get_service_details(&selected_service.name)
                .await?;
            self.display_service_details(&details);
            selected_details = Some(details);
        }

        // Action menu prompt
        let menu_prompt = format!("Options for {}", selected_service.name);

        // Action menu items (as shown in screenshot)
        let action_menu = vec![
            "[1] View service details (fingerprint, publication time)".to_string(),
            "[2] Export proto files".to_string(),
            "[3] Add to configuration file".to_string(),
        ];

        let action_choice = match user_interface
            .select_from_list(&action_menu, &menu_prompt)
            .await
        {
            Ok(choice) => choice,
            Err(err) if Self::is_operation_cancelled(&err) => {
                return Ok(CommandResult::Success("Operation cancelled".to_string()));
            }
            Err(err) => return Err(err),
        };

        match action_choice {
            0 => {
                if let Some(details) = selected_details.as_ref() {
                    self.display_service_details(details);
                } else {
                    let details = service_discovery
                        .get_service_details(&selected_service.name)
                        .await?;
                    self.display_service_details(&details);
                }
                Ok(CommandResult::Success(
                    "Service details displayed".to_string(),
                ))
            }
            1 => {
                // Export proto files
                self.export_proto_files(selected_service, &service_discovery, &config_manager)
                    .await?;
                Ok(CommandResult::Success("Proto files exported".to_string()))
            }
            2 => {
                // Add to configuration file - core flow of reuse architecture
                self.add_to_config_with_validation(selected_service, context)
                    .await
            }
            _ => Ok(CommandResult::Success("Invalid choice".to_string())),
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        // Components needed for Discovery command
        vec![
            ComponentType::ServiceDiscovery, // Core service discovery
            ComponentType::UserInterface,    // User interface
            ComponentType::ConfigManager,    // Configuration management
            ComponentType::DependencyResolver,
            ComponentType::NetworkValidator,
            ComponentType::FingerprintValidator,
        ]
    }

    fn name(&self) -> &str {
        "discovery"
    }

    fn description(&self) -> &str {
        "Discover available Actor services in the network (Reuse architecture + check-first)"
    }
}

impl DiscoveryCommand {
    pub fn new(filter: Option<String>, verbose: bool, auto_install: bool) -> Self {
        Self {
            filter,
            verbose,
            auto_install,
        }
    }

    // Create from clap Args
    pub fn from_args(args: &DiscoveryCommand) -> Self {
        DiscoveryCommand {
            filter: args.filter.clone(),
            verbose: args.verbose,
            auto_install: args.auto_install,
        }
    }

    /// Create service filter
    fn create_service_filter(&self) -> Option<crate::core::ServiceFilter> {
        self.filter
            .as_ref()
            .map(|pattern| crate::core::ServiceFilter {
                name_pattern: Some(pattern.clone()),
                version_range: None,
                tags: None,
            })
    }

    fn is_operation_cancelled(err: &anyhow::Error) -> bool {
        matches!(
            err.downcast_ref::<ActrCliError>(),
            Some(ActrCliError::OperationCancelled)
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn validate_dependency(
        &self,
        service: &ServiceInfo,
        dependency_spec: &DependencySpec,
        expected_fingerprint: Option<&str>,
        check_conflicts: bool,
        existing_specs: &[DependencySpec],
        dependency_resolver: &std::sync::Arc<dyn DependencyResolver>,
        service_discovery: &std::sync::Arc<dyn ServiceDiscovery>,
        network_validator: &std::sync::Arc<dyn NetworkValidator>,
        fingerprint_validator: &std::sync::Arc<dyn FingerprintValidator>,
    ) -> Result<()> {
        println!();
        println!("🔍 Validating dependency...");

        let mut failures = Vec::new();

        match service_discovery
            .check_service_availability(&service.name)
            .await
        {
            Ok(status) => {
                if status.is_available {
                    println!("  ├─ ✅ Service availability");
                } else {
                    println!("  ├─ ❌ Service availability");
                    failures.push(format!("Service '{}' not found in registry", service.name));
                }
            }
            Err(e) => {
                println!("  ├─ ❌ Service availability");
                failures.push(format!("Service availability check failed: {e}"));
            }
        }

        match network_validator
            .check_connectivity(&service.name, &NetworkCheckOptions::default())
            .await
        {
            Ok(connectivity) => {
                if connectivity.is_reachable {
                    println!("  ├─ ✅ Network connectivity");
                } else {
                    println!("  ├─ ❌ Network connectivity");
                    let detail = connectivity.error.as_deref().unwrap_or("unknown error");
                    failures.push(format!(
                        "Network connectivity failed for '{}': {}",
                        service.name, detail
                    ));
                }
            }
            Err(e) => {
                println!("  ├─ ❌ Network connectivity");
                failures.push(format!("Network connectivity check failed: {e}"));
            }
        }

        if let Some(expected_fingerprint) = expected_fingerprint.filter(|fp| !fp.is_empty()) {
            match fingerprint_validator
                .compute_service_fingerprint(service)
                .await
            {
                Ok(actual) => {
                    let expected = Fingerprint {
                        algorithm: actual.algorithm.clone(),
                        value: expected_fingerprint.to_string(),
                    };
                    let is_valid = fingerprint_validator
                        .verify_fingerprint(&expected, &actual)
                        .await
                        .unwrap_or(false);
                    if is_valid {
                        println!("  ├─ ✅ Fingerprint match");
                    } else {
                        println!("  ├─ ❌ Fingerprint match");
                        failures.push(format!("Fingerprint mismatch for '{}'", service.name));
                    }
                }
                Err(e) => {
                    println!("  ├─ ❌ Fingerprint check");
                    failures.push(format!("Fingerprint check failed: {e}"));
                }
            }
        } else {
            println!("  ├─ ⚠️  Fingerprint missing; skipping check");
        }

        if check_conflicts {
            let mut resolved = Vec::with_capacity(existing_specs.len() + 1);
            for spec in existing_specs {
                resolved.push(ResolvedDependency {
                    spec: spec.clone(),
                    fingerprint: spec.fingerprint.clone().unwrap_or_default(),
                    proto_files: Vec::new(),
                });
            }
            resolved.push(ResolvedDependency {
                spec: dependency_spec.clone(),
                fingerprint: dependency_spec.fingerprint.clone().unwrap_or_default(),
                proto_files: Vec::new(),
            });

            match dependency_resolver.check_conflicts(&resolved).await {
                Ok(conflicts) => {
                    if conflicts.is_empty() {
                        println!("  ├─ ✅ Dependency conflicts");
                    } else {
                        println!("  ├─ ❌ Dependency conflicts");
                        let details = conflicts
                            .iter()
                            .map(|conflict| conflict.description.clone())
                            .collect::<Vec<_>>()
                            .join(", ");
                        failures.push(format!("Dependency conflicts: {details}"));
                    }
                }
                Err(e) => {
                    println!("  ├─ ❌ Dependency conflicts");
                    failures.push(format!("Dependency conflict check failed: {e}"));
                }
            }
        } else {
            println!("  ├─ ⚠️  Dependency conflict check skipped (already configured)");
        }

        if failures.is_empty() {
            println!("  └─ ✅ Validation passed");
            Ok(())
        } else {
            println!("  └─ ❌ Validation failed");
            Err(ActrCliError::ValidationFailed {
                details: failures.join("; "),
            }
            .into())
        }
    }

    /// Display services table
    fn display_services_table(&self, services: &[ServiceInfo]) {
        println!();
        // Total width limit is 160
        const TOTAL_MAX_WIDTH: usize = 160;
        // Border and separator overhead
        const BORDER_OVERHEAD: usize = 7;

        // Calculate the maximum width of each column
        let name_width = services
            .iter()
            .map(|s| s.name.chars().count())
            .max()
            .unwrap_or(0)
            .max("Service Name".len());

        let tags_width = services
            .iter()
            .map(|s| s.tags.join(", ").chars().count())
            .max()
            .unwrap_or(0)
            .max("Tags".len());

        let desc_width = services
            .iter()
            .map(|s| {
                s.description
                    .as_deref()
                    .unwrap_or("No description")
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(0)
            .max("Description".len());

        let name_w = name_width;
        let tags_w = tags_width;
        let mut desc_w = desc_width;

        // If the total width is exceeded, truncate the Description
        if name_w + tags_w + desc_w + BORDER_OVERHEAD > TOTAL_MAX_WIDTH {
            let available = TOTAL_MAX_WIDTH - BORDER_OVERHEAD;
            let used = name_w + tags_w;
            desc_w = available.saturating_sub(used).max(10); // Description 至少 10 字符
        }

        // Generate table header
        let top_border = format!(
            "┌─{}─┬─{}─┬─{}─┐",
            "─".repeat(name_w),
            "─".repeat(tags_w),
            "─".repeat(desc_w)
        );
        let header = format!(
            "│ {:width$} │ {:tags_w$} │ {:desc_w$} │",
            "Service Name",
            "Tags",
            "Description",
            width = name_w,
            tags_w = tags_w,
            desc_w = desc_w
        );
        let separator = format!(
            "├─{}─┼─{}─┼─{}─┤",
            "─".repeat(name_w),
            "─".repeat(tags_w),
            "─".repeat(desc_w)
        );
        let bottom_border = format!(
            "└─{}─┴─{}─┴─{}─┘",
            "─".repeat(name_w),
            "─".repeat(tags_w),
            "─".repeat(desc_w)
        );

        println!("{top_border}");
        println!("{header}");
        println!("{separator}");

        for service in services {
            let tags_str = service.tags.join(", ");
            let description = service
                .description
                .as_deref()
                .unwrap_or("No description")
                .chars()
                .take(desc_w)
                .collect::<String>();

            println!(
                "│ {:name_w$} │ {:tags_w$} │ {:desc_w$} │",
                service.name,
                tags_str.chars().take(tags_w).collect::<String>(),
                description,
                name_w = name_w,
                tags_w = tags_w,
                desc_w = desc_w
            );
        }

        println!("{bottom_border}");
        println!();
    }

    /// Display service info
    fn display_service_info(&self, service: &ServiceInfo) {
        println!("📋 Selected service: {}", service.name);
        if let Some(desc) = &service.description {
            println!("📝 Description: {desc}");
        }
        println!("🔐 Fingerprint: {}", service.fingerprint);
        let time = service
            .published_at
            .and_then(|published_at| chrono::DateTime::from_timestamp(published_at, 0))
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| "Unknown".to_string());
        println!("📅 Publication Time: {}", time);
        println!(
            "🏷️  Tags: {}",
            if service.tags.is_empty() {
                "(none)".to_string()
            } else {
                service.tags.join(", ")
            }
        );
        println!("📊 Methods count: {}", service.methods.len());
        println!();
    }

    #[allow(unused)]
    /// Display service details
    fn display_service_details(&self, details: &ServiceDetails) {
        println!("📖 {} Detailed Information:", details.info.name);
        println!("════════════════════════════════════════");
        self.display_service_info(&details.info);
        println!("📋 Available Methods:");
        if details.info.methods.is_empty() {
            println!("  (None)");
        } else {
            for method in &details.info.methods {
                println!(
                    "  • {}: {} → {}",
                    method.name, method.input_type, method.output_type
                );
            }
        }

        if !details.dependencies.is_empty() {
            println!();
            println!("🔗 Dependent Services:");
            for dep in &details.dependencies {
                println!("  • {dep}");
            }
        }

        println!();
        println!("📁 Proto Files:");
        if details.proto_files.is_empty() {
            println!("  (None)");
        } else {
            for proto in &details.proto_files {
                println!("  • {} ({} services)", proto.name, proto.services.len());
            }
        }

        println!();
    }

    /// Export proto files
    async fn export_proto_files(
        &self,
        service: &ServiceInfo,
        service_discovery: &std::sync::Arc<dyn ServiceDiscovery>,
        config_manager: &std::sync::Arc<dyn ConfigManager>,
    ) -> Result<()> {
        println!("📤 Exporting proto files for {}...", service.name);

        let proto_files = service_discovery.get_service_proto(&service.name).await?;

        let output_dir = config_manager
            .get_project_root()
            .join("exports")
            .join("remote")
            .join(&service.name);
        std::fs::create_dir_all(&output_dir)?;

        for proto in &proto_files {
            let file_path = output_dir.join(&proto.name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file_path, &proto.content)?;
            println!("✅ Exported: {}", file_path.display());
        }

        println!("🎉 Export completed, total {} files", proto_files.len());
        Ok(())
    }

    /// Add to configuration file - core flow of reuse architecture
    async fn add_to_config_with_validation(
        &self,
        service: &ServiceInfo,
        context: &CommandContext,
    ) -> Result<CommandResult> {
        let (
            config_manager,
            user_interface,
            dependency_resolver,
            service_discovery,
            network_validator,
            fingerprint_validator,
        ) = {
            let container = context.container.lock().unwrap();
            (
                container.get_config_manager()?,
                container.get_user_interface()?,
                container.get_dependency_resolver()?,
                container.get_service_discovery()?,
                container.get_network_validator()?,
                container.get_fingerprint_validator()?,
            )
        };

        // Convert to dependency spec
        let dependency_spec = DependencySpec {
            alias: service.name.clone(),
            actr_type: Some(service.actr_type.clone()),
            name: service.name.clone(),
            fingerprint: Some(service.fingerprint.clone()),
        };

        // Check if a dependency with the same name already exists
        let config = config_manager
            .load_config(
                config_manager
                    .get_project_root()
                    .join("Actr.toml")
                    .as_path(),
            )
            .await?;

        let existing_by_name = config.dependencies.iter().find(|dep| {
            dep.service
                .as_ref()
                .map(|s| s.name.as_str())
                .or_else(|| dep.actr_type.as_ref().map(|t| t.name.as_str()))
                .unwrap_or(dep.alias.as_str())
                == service.name
        });
        let existing_by_alias = config
            .dependencies
            .iter()
            .find(|dep| dep.alias == dependency_spec.alias);

        if let Some(existing) = existing_by_alias
            && existing
                .service
                .as_ref()
                .map(|s| s.name.as_str())
                .or_else(|| existing.actr_type.as_ref().map(|t| t.name.as_str()))
                .unwrap_or(existing.alias.as_str())
                != service.name
        {
            return Err(ActrCliError::Dependency {
                message: format!(
                    "Dependency alias '{}' already exists for '{}'",
                    existing.alias,
                    existing
                        .service
                        .as_ref()
                        .map(|s| s.name.as_str())
                        .or_else(|| existing.actr_type.as_ref().map(|t| t.name.as_str()))
                        .unwrap_or(existing.alias.as_str())
                ),
            }
            .into());
        }

        let should_update_config = existing_by_name.is_none();
        if let Some(existing) = existing_by_name {
            println!(
                "ℹ️  Dependency with name '{}' already exists (alias: '{}')",
                service.name, existing.alias
            );
            if let (Some(existing_fp), Some(discovered_fp)) = (
                existing.service.as_ref().map(|s| s.fingerprint.as_str()),
                dependency_spec.fingerprint.as_deref(),
            ) && existing_fp != discovered_fp
            {
                println!(
                    "⚠️  Fingerprint mismatch: config '{}' vs discovery '{}'",
                    existing_fp, discovered_fp
                );
            }
            println!("   Skipping configuration update");
        }

        let expected_fingerprint = existing_by_name
            .and_then(|dep| dep.service.as_ref().map(|s| s.fingerprint.clone()))
            .or_else(|| dependency_spec.fingerprint.clone());
        let existing_specs = dependency_resolver.resolve_spec(&config).await?;
        self.validate_dependency(
            service,
            &dependency_spec,
            expected_fingerprint.as_deref(),
            should_update_config,
            &existing_specs,
            &dependency_resolver,
            &service_discovery,
            &network_validator,
            &fingerprint_validator,
        )
        .await?;

        if should_update_config {
            println!("📝 Adding {} to configuration file...", service.name);
            let backup = config_manager.backup_config().await?;
            match config_manager.update_dependency(&dependency_spec).await {
                Ok(_) => {
                    config_manager.remove_backup(backup).await?;
                    println!("✅ Added {} to configuration file", service.name);
                }
                Err(e) => {
                    config_manager.restore_backup(backup).await?;
                    return Err(ActrCliError::Config {
                        message: format!("Configuration update failed: {e}"),
                    }
                    .into());
                }
            }
        }

        // Ask if user wants to install immediately
        println!();
        let should_install = if self.auto_install {
            true
        } else {
            user_interface
                .confirm("🤔 Install this dependency now?")
                .await?
        };

        if should_install {
            // Reuse install flow
            println!();
            println!("📦 Installing {}...", service.name);

            let install_pipeline = {
                let mut container = context.container.lock().unwrap();
                match container.get_install_pipeline() {
                    Ok(pipeline) => pipeline,
                    Err(_) => {
                        println!("ℹ️ Install pipeline is not implemented yet; skipping.");
                        return Ok(CommandResult::Success(
                            "Dependency added; install pending".to_string(),
                        ));
                    }
                }
            };

            match install_pipeline
                .install_dependencies(&[dependency_spec])
                .await
            {
                Ok(install_result) => {
                    println!("  ├─ 📦 Cache proto files ✅");
                    println!("  ├─ 🔒 Update lock file ✅");
                    println!("  └─ ✅ Installation complete");
                    println!();
                    println!("💡 Tip: Run 'actr gen' to generate the latest code");

                    Ok(CommandResult::Install(install_result))
                }
                Err(e) => {
                    eprintln!("❌ Installation failed: {e}");
                    Ok(CommandResult::Success(
                        "Dependency added but installation failed".to_string(),
                    ))
                }
            }
        } else {
            println!("✅ Dependency added to configuration file");
            println!("💡 Tip: Run 'actr install' to install dependencies");
            Ok(CommandResult::Success(
                "Dependency added to configuration".to_string(),
            ))
        }
    }
}

impl Default for DiscoveryCommand {
    fn default() -> Self {
        Self::new(None, false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_service_filter() {
        let cmd = DiscoveryCommand::new(Some("user-*".to_string()), false, false);
        let filter = cmd.create_service_filter();

        assert!(filter.is_some());
        let filter = filter.unwrap();
        assert_eq!(filter.name_pattern, Some("user-*".to_string()));
    }

    #[test]
    fn test_create_service_filter_none() {
        let cmd = DiscoveryCommand::new(None, false, false);
        let filter = cmd.create_service_filter();

        assert!(filter.is_none());
    }

    #[test]
    fn test_required_components() {
        let cmd = DiscoveryCommand::default();
        let components = cmd.required_components();

        // Discovery command requires validation components for check-first flow.
        assert!(components.contains(&ComponentType::ServiceDiscovery));
        assert!(components.contains(&ComponentType::UserInterface));
        assert!(components.contains(&ComponentType::ConfigManager));
        assert!(components.contains(&ComponentType::DependencyResolver));
        assert!(components.contains(&ComponentType::NetworkValidator));
        assert!(components.contains(&ComponentType::FingerprintValidator));
        assert!(!components.contains(&ComponentType::CacheManager));
        assert!(!components.contains(&ComponentType::ProtoProcessor));
    }
}
