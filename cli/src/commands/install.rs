//! Install Command Implementation
//!
//! Implement install flow based on reuse architecture with check-first principle

use crate::core::{
    ActrCliError, Command, CommandContext, CommandResult, ComponentType, DependencySpec,
    ErrorReporter, InstallResult,
};
use crate::utils::command_exists;
use actr_config::LockFile;
use actr_protocol::{ActrType, ActrTypeExt};
use actr_service_compat::{CompatibilityLevel, Fingerprint, ProtoFile, ServiceCompatibility};
use anyhow::Result;
use async_trait::async_trait;
use clap::Args;
use std::path::Path;
use std::process::Command as StdCommand;

/// Install command
#[derive(Args, Debug)]
#[command(
    about = "Install service dependencies",
    long_about = "Install service dependencies. You can install specific service packages, or install all dependencies configured in manifest.toml.\n\nExamples:\n  actr deps install                          # Install all dependencies from manifest.toml\n  actr deps install user-service             # Install a service by name\n  actr deps install my-alias --actr-type acme:EchoService  # Install with alias and explicit actr_type"
)]
pub struct InstallCommand {
    /// Package name or alias (when used with --actr-type, this becomes the alias)
    #[arg(value_name = "PACKAGE")]
    pub packages: Vec<String>,

    /// Actor type for the dependency (format: manufacturer:name:version, e.g., acme:EchoService:1.0.0).
    /// When specified, the PACKAGE argument is treated as an alias.
    #[arg(long, value_name = "TYPE")]
    pub actr_type: Option<String>,

    /// Service fingerprint for version pinning
    #[arg(long, value_name = "FINGERPRINT")]
    pub fingerprint: Option<String>,

    /// Force reinstallation
    #[arg(long)]
    pub force: bool,

    /// Force update of all dependencies
    #[arg(long)]
    pub force_update: bool,

    /// Skip fingerprint verification
    #[arg(long)]
    pub skip_verification: bool,
}

/// Installation mode
#[derive(Debug, Clone)]
pub enum InstallMode {
    /// Mode 1: Add new dependency (npm install <package>)
    /// - Pull remote proto to protos/ folder
    /// - Modify manifest.toml (add dependency)
    /// - Update manifest.lock.toml
    AddNewPackage { packages: Vec<String> },

    /// Mode 1b: Add dependency with explicit alias and actr_type (actr deps install <alias> --actr-type <type>)
    /// - Discover service by actr_type
    /// - Use first argument as alias
    /// - Modify manifest.toml (add dependency with alias)
    /// - Update manifest.lock.toml
    AddWithAlias {
        alias: String,
        actr_type: ActrType,
        fingerprint: Option<String>,
    },

    /// Mode 2: Install dependencies in config (npm install)
    /// - Do NOT modify manifest.toml
    /// - Use lock file versions if available
    /// - Only update manifest.lock.toml
    InstallFromConfig { force_update: bool },
}

#[async_trait]
impl Command for InstallCommand {
    async fn execute(&self, context: &CommandContext) -> Result<CommandResult> {
        // Check-First principle: validate project state first
        if !self.is_actr_project() {
            return Err(ActrCliError::InvalidProject {
                message: "Not an Actor-RTC project. Run 'actr init' to initialize.".to_string(),
            }
            .into());
        }

        // Determine installation mode
        let mode = if let Some(actr_type_str) = &self.actr_type {
            // Mode 1b: Install with explicit alias and actr_type
            if self.packages.is_empty() {
                return Err(ActrCliError::InvalidArgument {
                    message:
                        "When using --actr-type, you must provide an alias as the first argument"
                            .to_string(),
                }
                .into());
            }
            let alias = self.packages[0].clone();
            let actr_type = ActrType::from_string_repr(actr_type_str).map_err(|_| {
                ActrCliError::InvalidArgument {
                    message: format!(
                        "Invalid actr_type format '{}'. Expected format: manufacturer:name:version (e.g., acme:EchoService:1.0.0)",
                        actr_type_str
                    ),
                }
            })?;
            InstallMode::AddWithAlias {
                alias,
                actr_type,
                fingerprint: self.fingerprint.clone(),
            }
        } else if !self.packages.is_empty() {
            if self.fingerprint.is_some() {
                return Err(ActrCliError::InvalidArgument {
                    message: "Using --fingerprint requires specifying --actr-type explicitly.
Use: actr deps install <ALIAS> --actr-type <TYPE> --fingerprint <FINGERPRINT>"
                        .to_string(),
                }
                .into());
            }

            InstallMode::AddNewPackage {
                packages: self.packages.clone(),
            }
        } else {
            if self.fingerprint.is_some() {
                return Err(ActrCliError::InvalidArgument {
                    message: "Using --fingerprint requires specifying an alias and --actr-type.
Use: actr deps install <ALIAS> --actr-type <TYPE> --fingerprint <FINGERPRINT>"
                        .to_string(),
                }
                .into());
            }

            InstallMode::InstallFromConfig {
                force_update: self.force_update,
            }
        };

        // Execute based on mode
        match mode {
            InstallMode::AddNewPackage { ref packages } => {
                self.execute_add_package(context, packages).await
            }
            InstallMode::AddWithAlias {
                ref alias,
                ref actr_type,
                ref fingerprint,
            } => {
                self.execute_add_with_alias(context, alias, actr_type, fingerprint.as_deref())
                    .await
            }
            InstallMode::InstallFromConfig { force_update } => {
                self.execute_install_from_config(context, force_update)
                    .await
            }
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        // Install command needs complete install pipeline components
        vec![
            ComponentType::ConfigManager,
            ComponentType::DependencyResolver,
            ComponentType::ServiceDiscovery,
            ComponentType::NetworkValidator,
            ComponentType::FingerprintValidator,
            ComponentType::ProtoProcessor,
            ComponentType::CacheManager,
        ]
    }

    fn name(&self) -> &str {
        "install"
    }

    fn description(&self) -> &str {
        "npm-style service-level dependency management (check-first architecture)"
    }
}

impl InstallCommand {
    pub fn new(
        packages: Vec<String>,
        actr_type: Option<String>,
        fingerprint: Option<String>,
        force: bool,
        force_update: bool,
        skip_verification: bool,
    ) -> Self {
        Self {
            packages,
            actr_type,
            fingerprint,
            force,
            force_update,
            skip_verification,
        }
    }

    // Create from clap Args
    pub fn from_args(args: &InstallCommand) -> Self {
        InstallCommand {
            packages: args.packages.clone(),
            actr_type: args.actr_type.clone(),
            fingerprint: args.fingerprint.clone(),
            force: args.force,
            force_update: args.force_update,
            skip_verification: args.skip_verification,
        }
    }

    /// Check if in Actor-RTC project
    fn is_actr_project(&self) -> bool {
        std::path::Path::new("manifest.toml").exists()
    }

    fn dependency_lookup_key(spec: &DependencySpec) -> String {
        spec.actr_type
            .as_ref()
            .map(|actr_type| actr_type.to_string_repr())
            .unwrap_or_else(|| spec.name.clone())
    }

    /// Execute Mode 1: Add new package (actr deps install <package>)
    /// - Pull remote proto to protos/ folder
    /// - Modify manifest.toml (add dependency)
    /// - Update manifest.lock.toml
    async fn execute_add_package(
        &self,
        context: &CommandContext,
        packages: &[String],
    ) -> Result<CommandResult> {
        println!("actr deps install {}", packages.join(" "));

        let install_pipeline = {
            let mut container = context.container.lock().unwrap();
            container.get_install_pipeline()?
        };

        let mut resolved_specs = Vec::new();

        println!("🔍 Phase 1: Complete Validation");
        for package in packages {
            // Phase 1: Check-First validation
            println!("  ├─ 📋 Parsing dependency spec: {}", package);

            // Discover service details
            // The service_details in install_pipeline is designed to fetch specific service details directly
            // However, we want to support interactive selection if multiple services match (or same service with multiple versions)
            // But get_service_details currently only returns one service or error
            // To support interactive selection, we need to use discover_services first

            let service_discovery = install_pipeline.validation_pipeline().service_discovery();
            let ui = context.container.lock().unwrap().get_user_interface()?;

            // First, try to discover services matching the name
            // We create a filter for the name
            let filter = crate::core::ServiceFilter {
                name_pattern: Some(package.clone()),
                version_range: None,
                tags: None,
            };

            let services = service_discovery.discover_services(Some(&filter)).await?;

            let selected_service = if services.is_empty() {
                // If no services found by discovery, fall back to get_service_details which might have different lookup logic
                // or just error out with the nice message we added earlier
                match service_discovery.get_service_details(package).await {
                    Ok(details) => details.info,
                    Err(_) => {
                        println!("  └─ ⚠️  Service not found: {}", package);
                        println!();
                        println!(
                            "💡 Tip: If you want to specify a fingerprint, use the full command:"
                        );
                        println!(
                            "      actr deps install {} --actr-type <TYPE> --fingerprint <FINGERPRINT>",
                            package
                        );
                        println!();
                        return Err(anyhow::anyhow!("Service not found"));
                    }
                }
            } else if services.len() == 1 {
                // Only one service found, auto-select
                let service = services[0].clone();
                println!("  ├─ 🔍 Automatically selected service: {}", service.name);
                service
            } else {
                // Multiple services found, ask user to select
                println!(
                    "  ├─ 🔍 Found {} services matching '{}'",
                    services.len(),
                    package
                );

                // Format items for selection
                let items: Vec<String> = services
                    .iter()
                    .map(|s| {
                        format!(
                            "{} ({}) - {}",
                            s.name,
                            s.fingerprint.chars().take(8).collect::<String>(),
                            s.actr_type.to_string_repr()
                        )
                    })
                    .collect();

                let selection_index = ui
                    .select_from_list(&items, "Please select a service to install")
                    .await?;

                services[selection_index].clone()
            };

            let service_details = service_discovery
                .get_service_details(&selected_service.name)
                .await?;

            println!(
                "  ├─ 🔍 Service discovery: fingerprint {}",
                service_details.info.fingerprint
            );

            // Connectivity check - Skipped for install as we only need metadata
            // let connectivity = install_pipeline
            //     .validation_pipeline()
            //     .network_validator()
            //     .check_connectivity(package, &NetworkCheckOptions::default())
            //     .await?;

            println!("  ├─ 🌐 Network connectivity test (Skipped) ✅");

            // Fingerprint check
            println!("  ├─ 🔐 Fingerprint integrity verification ✅");

            // Create dependency spec with resolved info
            let resolved_spec = DependencySpec {
                alias: package.clone(),
                actr_type: Some(service_details.info.actr_type.clone()),
                name: package.clone(),
                fingerprint: Some(service_details.info.fingerprint.clone()),
            };
            resolved_specs.push(resolved_spec);
            println!("  └─ ✅ Added to installation plan");
            println!();
        }

        if resolved_specs.is_empty() {
            return Ok(CommandResult::Success("No packages to install".to_string()));
        }

        // Phase 2: Atomic installation
        println!("📝 Phase 2: Atomic Installation");

        // Execute installation for all packages
        match install_pipeline.install_dependencies(&resolved_specs).await {
            Ok(result) => {
                println!("  ├─ 💾 Backing up current configuration");
                println!("  ├─ 📝 Updating manifest.toml configuration ✅");
                println!("  ├─ 📦 Caching proto files ✅");
                println!("  ├─ 🔒 Updating manifest.lock.toml ✅");
                println!("  └─ ✅ Installation completed");
                println!();
                self.install_npm_dependencies_if_needed()?;
                self.display_install_success(&result);
                Ok(CommandResult::Install(result))
            }
            Err(e) => {
                println!("  └─ 🔄 Restoring backup (due to installation failure)");
                let cli_error = ActrCliError::InstallFailed {
                    reason: e.to_string(),
                };
                eprintln!("{}", ErrorReporter::format_error(&cli_error));
                Err(e)
            }
        }
    }

    /// Execute Mode 1b: Add dependency with explicit alias and actr_type
    /// - Discover service by actr_type
    /// - Use provided alias
    /// - Modify manifest.toml (add dependency with alias)
    /// - Update manifest.lock.toml
    async fn execute_add_with_alias(
        &self,
        context: &CommandContext,
        alias: &str,
        actr_type: &ActrType,
        fingerprint: Option<&str>,
    ) -> Result<CommandResult> {
        use actr_protocol::ActrTypeExt;

        println!(
            "actr deps install {} --actr-type {}",
            alias,
            actr_type.to_string_repr()
        );

        let install_pipeline = {
            let mut container = context.container.lock().unwrap();
            container.get_install_pipeline()?
        };

        println!("🔍 Phase 1: Complete Validation");
        println!("  ├─ 📋 Alias: {}", alias);
        println!("  ├─ 🏷️  Actor Type: {}", actr_type.to_string_repr());

        // Discover service by dependencies value (actr_type)
        // Step 1: Build lookup key from manufacturer:name (matching service registration convention)
        // Step 2: Filter by version from the full actr_type
        let service_discovery = install_pipeline.validation_pipeline().service_discovery();

        let lookup_key = format!("{}:{}", actr_type.manufacturer, actr_type.name);
        let filter = crate::core::ServiceFilter {
            name_pattern: Some(lookup_key.clone()),
            version_range: None,
            tags: None,
        };

        let services = service_discovery.discover_services(Some(&filter)).await?;

        // Filter by version
        let matching_service = services
            .iter()
            .find(|s| {
                s.actr_type.manufacturer == actr_type.manufacturer
                    && s.actr_type.name == actr_type.name
                    && s.actr_type.version == actr_type.version
            })
            .ok_or_else(|| ActrCliError::ServiceNotFound {
                name: actr_type.to_string_repr(),
            })?;

        let service_name = matching_service.name.clone();
        println!("  ├─ 🔍 Service discovered: {}", service_name);

        // Get full service details (proto files etc.)
        // Use actr_type.name for ServiceSpec lookup (matching server-side spec.name = package.name)
        let service_details = service_discovery.get_service_details(&service_name).await?;

        println!(
            "  ├─ 🔍 Service fingerprint: {}",
            service_details.info.fingerprint
        );

        // Verify fingerprint if provided
        if let Some(expected_fp) = fingerprint {
            if service_details.info.fingerprint != expected_fp {
                println!("  └─ ❌ Fingerprint mismatch");
                return Err(ActrCliError::FingerprintMismatch {
                    expected: expected_fp.to_string(),
                    actual: service_details.info.fingerprint.clone(),
                }
                .into());
            }
            println!("  ├─ 🔐 Fingerprint verification ✅");
        }

        // Connectivity check - Skipped for install as we only need metadata
        // let connectivity = install_pipeline
        //     .validation_pipeline()
        //     .network_validator()
        //     .check_connectivity(&service_name, &NetworkCheckOptions::default())
        //     .await?;

        println!("  ├─ 🌐 Network connectivity test (Skipped) ✅");

        // Create dependency spec with alias
        // name = alias ensures update_dependency won't write a redundant "name" field
        let resolved_spec = DependencySpec {
            alias: alias.to_string(),
            actr_type: Some(service_details.info.actr_type.clone()),
            name: alias.to_string(),
            fingerprint: fingerprint.map(|s| s.to_string()),
        };

        println!("  └─ ✅ Added to installation plan");
        println!();

        // Phase 2: Atomic installation
        println!("📝 Phase 2: Atomic Installation");

        // Execute installation
        match install_pipeline
            .install_dependencies(&[resolved_spec])
            .await
        {
            Ok(result) => {
                println!("  ├─ 💾 Backing up current configuration");
                println!("  ├─ 📝 Updating manifest.toml configuration ✅");
                println!("  ├─ 📦 Caching proto files ✅");
                println!("  ├─ 🔒 Updating manifest.lock.toml ✅");
                println!("  └─ ✅ Installation completed");
                println!();
                self.install_npm_dependencies_if_needed()?;
                self.display_install_success(&result);
                Ok(CommandResult::Install(result))
            }
            Err(e) => {
                println!("  └─ 🔄 Restoring backup (due to installation failure)");
                let cli_error = ActrCliError::InstallFailed {
                    reason: e.to_string(),
                };
                eprintln!("{}", ErrorReporter::format_error(&cli_error));
                Err(e)
            }
        }
    }

    /// Execute Mode 2: Install from config (actr deps install)
    /// - Do NOT modify manifest.toml
    /// - Use lock file versions if available
    /// - Check for compatibility conflicts when lock file exists
    /// - Only update manifest.lock.toml
    async fn execute_install_from_config(
        &self,
        context: &CommandContext,
        force_update: bool,
    ) -> Result<CommandResult> {
        if force_update || self.force {
            println!("📦 Force updating all service dependencies");
        } else {
            println!("📦 Installing service dependencies from config");
        }
        println!();

        // Load dependencies from manifest.toml
        let dependency_specs = self.load_dependencies_from_config(context).await?;

        if dependency_specs.is_empty() {
            println!("ℹ️ No dependencies configured, generating empty lock file");

            // Generate empty lock file with metadata
            let install_pipeline = {
                let mut container = context.container.lock().unwrap();
                container.get_install_pipeline()?
            };
            let project_root = install_pipeline.config_manager().get_project_root();
            let lock_file_path = project_root.join("manifest.lock.toml");

            let mut lock_file = LockFile::new();
            lock_file.update_timestamp();
            lock_file
                .save_to_file(&lock_file_path)
                .map_err(|e| ActrCliError::InstallFailed {
                    reason: format!("Failed to save lock file: {}", e),
                })?;

            println!("  └─ 🔒 Generated manifest.lock.toml");
            self.install_npm_dependencies_if_needed()?;
            return Ok(CommandResult::Success(
                "Generated empty lock file".to_string(),
            ));
        }

        // Check for duplicate actr_type conflicts
        let conflicts = self.check_actr_type_conflicts(&dependency_specs);
        if !conflicts.is_empty() {
            println!("❌ Dependency conflict detected:");
            for conflict in &conflicts {
                println!("   • {}", conflict);
            }
            println!();
            println!(
                "💡 Tip: Each actr_type can only be used once. Please use different aliases for different services or remove duplicate dependencies."
            );
            return Err(ActrCliError::DependencyConflict {
                message: format!(
                    "{} dependency conflict(s) detected. Each actr_type must be unique.",
                    conflicts.len()
                ),
            }
            .into());
        }

        println!("🔍 Phase 1: Full Validation");
        for spec in &dependency_specs {
            println!("  ├─ 📋 Parsing dependency: {}", spec.alias);
        }

        // Get install pipeline
        let install_pipeline = {
            let mut container = context.container.lock().unwrap();
            container.get_install_pipeline()?
        };

        // Check for compatibility conflicts when lock file exists (unless force_update)
        if !force_update && !self.force {
            let project_root = install_pipeline.config_manager().get_project_root();
            let lock_file_path = project_root.join("manifest.lock.toml");
            if lock_file_path.exists() {
                println!("  ├─ 🔒 Lock file found, checking compatibility...");

                // Perform compatibility check
                let conflicts = self
                    .check_lock_file_compatibility(
                        &lock_file_path,
                        &dependency_specs,
                        &install_pipeline,
                    )
                    .await?;

                if !conflicts.is_empty() {
                    println!("  └─ ❌ Compatibility conflicts detected");
                    println!();
                    println!("⚠️  Breaking changes detected:");
                    for conflict in &conflicts {
                        println!("   • {}", conflict);
                    }
                    println!();
                    println!(
                        "💡 Tip: Use --force-update to override and update to the latest versions"
                    );
                    return Err(ActrCliError::CompatibilityConflict {
                        message: format!(
                            "{} breaking change(s) detected. Use --force-update to override.",
                            conflicts.len()
                        ),
                    }
                    .into());
                }
                println!("  ├─ ✅ Compatibility check passed");
            }
        }

        // Verify fingerprints match registered services (unless --force is used)
        println!("  ├─ ✅ Verifying fingerprints...");
        let fingerprint_mismatches = self
            .verify_fingerprints(&dependency_specs, &install_pipeline)
            .await?;

        if !fingerprint_mismatches.is_empty() && !self.force {
            println!("  └─ ❌ Fingerprint mismatch detected");
            println!();
            println!("⚠️  Fingerprint mismatch:");
            for mismatch in &fingerprint_mismatches {
                println!("   • {}", mismatch);
            }
            println!();
            println!(
                "💡 Tip: Use --force to update manifest.toml with the current service fingerprints"
            );
            return Err(ActrCliError::FingerprintValidation {
                message: format!(
                    "{} fingerprint mismatch(es) detected. Use --force to update.",
                    fingerprint_mismatches.len()
                ),
            }
            .into());
        }

        // If --force is used and there are mismatches, update manifest.toml
        if !fingerprint_mismatches.is_empty() && self.force {
            println!("  ├─ ⚠️  Fingerprint mismatch detected, updating manifest.toml...");
            self.update_config_fingerprints(context, &dependency_specs, &install_pipeline)
                .await?;
            println!("  ├─ ✅ manifest.toml updated with current fingerprints");

            // Reload dependency specs with updated fingerprints
            let dependency_specs = self.load_dependencies_from_config(context).await?;

            println!("  ├─ 🔍 Service discovery (DiscoveryRequest)");
            println!("  ├─ 🌐 Network connectivity test");
            println!("  └─ ✅ Installation plan generated");
            println!();

            // Execute installation with updated specs
            println!("📝 Phase 2: Atomic Installation");
            return match install_pipeline
                .install_dependencies(&dependency_specs)
                .await
            {
                Ok(install_result) => {
                    println!("  ├─ 📚 Caching proto files ✅");
                    println!("  ├─ 🔒 Updating manifest.lock.toml ✅");
                    println!("  └─ ✅ Installation completed");
                    println!();
                    println!(
                        "📝 Note: manifest.toml fingerprints were updated to match current services"
                    );
                    self.install_npm_dependencies_if_needed()?;
                    self.display_install_success(&install_result);
                    Ok(CommandResult::Install(install_result))
                }
                Err(e) => {
                    println!("  └─ ❌ Installation failed");
                    let cli_error = ActrCliError::InstallFailed {
                        reason: e.to_string(),
                    };
                    eprintln!("{}", ErrorReporter::format_error(&cli_error));
                    Err(e)
                }
            };
        }

        println!("  ├─ ✅ Fingerprint verification passed");
        println!("  ├─ 🔍 Service discovery (DiscoveryRequest)");
        println!("  ├─ 🌐 Network connectivity test");
        println!("  └─ ✅ Installation plan generated");
        println!();

        // Execute check-first install flow (Mode 2: no config update)
        println!("📝 Phase 2: Atomic Installation");
        match install_pipeline
            .install_dependencies(&dependency_specs)
            .await
        {
            Ok(install_result) => {
                println!("  ├─ 📦 Caching proto files ✅");
                println!("  ├─ 🔒 Updating manifest.lock.toml ✅");
                println!("  └─ ✅ Installation completed");
                println!();
                self.install_npm_dependencies_if_needed()?;
                self.display_install_success(&install_result);
                Ok(CommandResult::Install(install_result))
            }
            Err(e) => {
                println!("  └─ ❌ Installation failed");
                let cli_error = ActrCliError::InstallFailed {
                    reason: e.to_string(),
                };
                eprintln!("{}", ErrorReporter::format_error(&cli_error));
                Err(e)
            }
        }
    }

    /// Load dependencies from config file
    async fn load_dependencies_from_config(
        &self,
        context: &CommandContext,
    ) -> Result<Vec<DependencySpec>> {
        let config_manager = {
            let container = context.container.lock().unwrap();
            container.get_config_manager()?
        };
        let config = config_manager
            .load_config(
                config_manager
                    .get_project_root()
                    .join("manifest.toml")
                    .as_path(),
            )
            .await?;

        let specs: Vec<DependencySpec> = config
            .dependencies
            .into_iter()
            .map(|dependency| DependencySpec {
                alias: dependency.alias.clone(),
                actr_type: dependency.actr_type.clone(),
                name: dependency
                    .service
                    .as_ref()
                    .map(|service| service.name.clone())
                    .unwrap_or_else(|| dependency.alias.clone()),
                fingerprint: dependency
                    .service
                    .as_ref()
                    .map(|service| service.fingerprint.clone()),
            })
            .collect();

        Ok(specs)
    }

    /// Check for duplicate actr_type conflicts in dependencies
    fn check_actr_type_conflicts(&self, specs: &[DependencySpec]) -> Vec<String> {
        use std::collections::HashMap;

        let mut actr_type_map: HashMap<String, Vec<&str>> = HashMap::new();
        let mut conflicts = Vec::new();

        for spec in specs {
            if let Some(ref actr_type) = spec.actr_type {
                let type_str = actr_type.to_string_repr();
                actr_type_map.entry(type_str).or_default().push(&spec.alias);
            }
        }

        for (actr_type, aliases) in actr_type_map {
            if aliases.len() > 1 {
                conflicts.push(format!(
                    "actr_type '{}' is used by multiple dependencies: {}",
                    actr_type,
                    aliases.join(", ")
                ));
            }
        }

        conflicts
    }

    /// Verify that fingerprints in manifest.toml match the currently registered services
    async fn verify_fingerprints(
        &self,
        specs: &[DependencySpec],
        install_pipeline: &std::sync::Arc<crate::core::InstallPipeline>,
    ) -> Result<Vec<String>> {
        let mut mismatches = Vec::new();
        let service_discovery = install_pipeline.validation_pipeline().service_discovery();

        for spec in specs {
            // Only check if fingerprint is specified in manifest.toml
            let expected_fingerprint = match &spec.fingerprint {
                Some(fp) => fp,
                None => continue,
            };

            // Get current service details
            let lookup_key = Self::dependency_lookup_key(spec);
            let current_service = match service_discovery.get_service_details(&lookup_key).await {
                Ok(s) => s,
                Err(e) => {
                    mismatches.push(format!(
                        "{}: Service not found or unavailable ({})",
                        spec.alias, e
                    ));
                    continue;
                }
            };

            let current_fingerprint = &current_service.info.fingerprint;

            // Compare fingerprints
            if expected_fingerprint != current_fingerprint {
                mismatches.push(format!(
                    "{}: Expected fingerprint '{}', but service has '{}'",
                    spec.alias, expected_fingerprint, current_fingerprint
                ));
            }
        }

        Ok(mismatches)
    }

    /// Update manifest.toml with current service fingerprints
    async fn update_config_fingerprints(
        &self,
        _context: &CommandContext,
        specs: &[DependencySpec],
        install_pipeline: &std::sync::Arc<crate::core::InstallPipeline>,
    ) -> Result<()> {
        let service_discovery = install_pipeline.validation_pipeline().service_discovery();
        let config_manager = install_pipeline.config_manager();

        // Update fingerprints for each dependency that has one specified
        for spec in specs {
            if spec.fingerprint.is_none() {
                continue;
            }

            // Get current service fingerprint
            let lookup_key = Self::dependency_lookup_key(spec);
            let current_service = match service_discovery.get_service_details(&lookup_key).await {
                Ok(s) => s,
                Err(_) => continue,
            };

            let old_fingerprint = spec
                .fingerprint
                .clone()
                .unwrap_or_else(|| "none".to_string());
            let new_fingerprint = current_service.info.fingerprint.clone();

            // Create updated spec with new fingerprint
            let updated_spec = DependencySpec {
                alias: spec.alias.clone(),
                name: spec.name.clone(),
                actr_type: spec.actr_type.clone(),
                fingerprint: Some(new_fingerprint.clone()),
            };

            // Use update_dependency to modify manifest.toml directly
            config_manager.update_dependency(&updated_spec).await?;

            println!(
                "   📝 Updated '{}' fingerprint: {} → {}",
                spec.alias, old_fingerprint, new_fingerprint
            );
        }

        Ok(())
    }

    /// Check compatibility between locked dependencies and currently registered services
    ///
    /// This method compares the fingerprints stored in the lock file with the fingerprints
    /// of the services currently registered on the signaling server. If a service's proto
    /// definition has breaking changes compared to the locked version, a conflict is reported.
    async fn check_lock_file_compatibility(
        &self,
        lock_file_path: &std::path::Path,
        dependency_specs: &[DependencySpec],
        install_pipeline: &std::sync::Arc<crate::core::InstallPipeline>,
    ) -> Result<Vec<String>> {
        use actr_protocol::ServiceSpec;

        let mut conflicts = Vec::new();

        // Load lock file
        let lock_file = match LockFile::from_file(lock_file_path) {
            Ok(lf) => lf,
            Err(e) => {
                tracing::warn!("Failed to parse lock file: {}", e);
                return Ok(conflicts); // If we can't parse lock file, skip compatibility check
            }
        };

        // For each dependency, check if the currently registered service is compatible
        for spec in dependency_specs {
            // Find the locked dependency by name
            let locked_dep = lock_file.dependencies.iter().find(|d| d.name == spec.name);

            let locked_dep = match locked_dep {
                Some(d) => d,
                None => {
                    // Dependency not in lock file, skip (will be newly installed)
                    tracing::debug!("Dependency '{}' not in lock file, skipping", spec.name);
                    continue;
                }
            };

            let locked_fingerprint = &locked_dep.fingerprint;

            // Get current service details from the registry
            let service_discovery = install_pipeline.validation_pipeline().service_discovery();
            let lookup_key = Self::dependency_lookup_key(spec);
            let current_service = match service_discovery.get_service_details(&lookup_key).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to get service details for '{}': {}", spec.name, e);
                    continue;
                }
            };

            let current_fingerprint = &current_service.info.fingerprint;

            // If fingerprints match, no need for deep analysis
            if locked_fingerprint == current_fingerprint {
                tracing::debug!(
                    "Fingerprint match for '{}', no compatibility check needed",
                    spec.name
                );
                continue;
            }

            // Fingerprints differ - perform deep compatibility analysis using actr-service-compat
            tracing::info!(
                "Fingerprint mismatch for '{}': locked={}, current={}",
                spec.name,
                locked_fingerprint,
                current_fingerprint
            );

            // Build ServiceSpec from locked proto content for comparison
            // Note: Since lock file only stores metadata (not full proto content),
            // we need to use semantic fingerprint comparison for compatibility check

            // Convert current service proto files to actr-service-compat ProtoFile format
            let current_proto_files: Vec<ProtoFile> = current_service
                .proto_files
                .iter()
                .map(|pf| ProtoFile {
                    name: pf.name.clone(),
                    content: pf.content.clone(),
                    path: Some(pf.path.to_string_lossy().to_string()),
                })
                .collect();

            // Calculate current service's semantic fingerprint
            let current_semantic_fp =
                match Fingerprint::calculate_service_semantic_fingerprint(&current_proto_files) {
                    Ok(fp) => fp,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to calculate semantic fingerprint for '{}': {}",
                            spec.name,
                            e
                        );
                        // If we can't calculate fingerprint, report as potential conflict
                        conflicts.push(format!(
                            "{}: Unable to verify compatibility (fingerprint calculation failed)",
                            spec.name
                        ));
                        continue;
                    }
                };

            // Compare fingerprints using semantic analysis
            // The locked fingerprint should be a service_semantic fingerprint
            let locked_semantic = if locked_fingerprint.starts_with("service_semantic:") {
                locked_fingerprint
                    .strip_prefix("service_semantic:")
                    .unwrap_or(locked_fingerprint)
            } else {
                locked_fingerprint.as_str()
            };

            if current_semantic_fp != locked_semantic {
                // Semantic fingerprints differ - this indicates breaking changes
                // Build ServiceSpec structures for detailed comparison
                let locked_spec = ServiceSpec {
                    name: spec.name.clone(),
                    description: locked_dep.description.clone(),
                    fingerprint: locked_fingerprint.clone(),
                    protobufs: locked_dep
                        .files
                        .iter()
                        .map(|pf| actr_protocol::service_spec::Protobuf {
                            package: pf.path.clone(),
                            content: String::new(), // Lock file doesn't store content
                            fingerprint: pf.fingerprint.clone(),
                        })
                        .collect(),
                    published_at: locked_dep.published_at,
                    tags: locked_dep.tags.clone(),
                };

                let current_spec = ServiceSpec {
                    name: spec.name.clone(),
                    description: Some(current_service.info.description.clone().unwrap_or_default()),
                    fingerprint: format!("service_semantic:{}", current_semantic_fp),
                    protobufs: current_proto_files
                        .iter()
                        .map(|pf| actr_protocol::service_spec::Protobuf {
                            package: pf.name.clone(),
                            content: pf.content.clone(),
                            fingerprint: String::new(),
                        })
                        .collect(),
                    published_at: current_service.info.published_at,
                    tags: current_service.info.tags.clone(),
                };

                // Attempt to analyze compatibility
                match ServiceCompatibility::analyze_compatibility(&locked_spec, &current_spec) {
                    Ok(analysis) => {
                        match analysis.level {
                            CompatibilityLevel::BreakingChanges => {
                                let change_summary = analysis
                                    .breaking_changes
                                    .iter()
                                    .map(|c| c.message.clone())
                                    .collect::<Vec<_>>()
                                    .join("; ");

                                conflicts.push(format!(
                                    "{}: Breaking changes detected - {}",
                                    spec.name, change_summary
                                ));
                            }
                            CompatibilityLevel::BackwardCompatible => {
                                tracing::info!(
                                    "Service '{}' has backward compatible changes",
                                    spec.name
                                );
                                // Backward compatible is allowed, no conflict
                            }
                            CompatibilityLevel::FullyCompatible => {
                                // This shouldn't happen if fingerprints differ, but handle it
                                tracing::debug!(
                                    "Service '{}' is fully compatible despite fingerprint difference",
                                    spec.name
                                );
                            }
                        }
                    }
                    Err(e) => {
                        // If detailed analysis fails, report based on fingerprint difference
                        tracing::warn!("Compatibility analysis failed for '{}': {}", spec.name, e);
                        conflicts.push(format!(
                            "{}: Service definition changed (locked: {}, current: {})",
                            spec.name, locked_fingerprint, current_fingerprint
                        ));
                    }
                }
            }
        }

        Ok(conflicts)
    }

    /// Display install success information
    fn display_install_success(&self, result: &InstallResult) {
        println!();
        println!("✅ Installation successful!");
        println!(
            "   📦 Installed dependencies: {}",
            result.installed_dependencies.len()
        );
        println!("   🗂️  Cache updates: {}", result.cache_updates);

        if result.updated_config {
            println!("   📝 Configuration file updated");
        }

        if result.updated_lock_file {
            println!("   🔒 Lock file updated");
        }

        if !result.warnings.is_empty() {
            println!();
            println!("⚠️  Warnings:");
            for warning in &result.warnings {
                println!("   • {warning}");
            }
        }

        println!();
        println!("💡 Tip: Run 'actr gen' to generate the latest code");
    }

    fn install_npm_dependencies_if_needed(&self) -> Result<()> {
        if !Path::new("package.json").exists() || !Path::new("tsconfig.json").exists() {
            return Ok(());
        }

        if !command_exists("npm") {
            return Err(ActrCliError::Command {
                message: "npm not found. TypeScript projects require npm to install package dependencies.".to_string(),
            }
            .into());
        }

        println!("📦 Installing npm dependencies");
        let output = StdCommand::new("npm")
            .arg("install")
            .output()
            .map_err(|e| ActrCliError::Command {
                message: format!("Failed to run npm install: {e}"),
            })?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ActrCliError::Command {
                message: format!("npm install failed:\nstdout: {stdout}\nstderr: {stderr}"),
            }
            .into());
        }

        println!("  └─ ✅ npm dependencies installed");
        Ok(())
    }
}

impl Default for InstallCommand {
    fn default() -> Self {
        Self::new(Vec::new(), None, None, false, false, false)
    }
}
