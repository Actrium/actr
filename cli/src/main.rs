//! ACTR-CLI - Actor-RTC Command Line Tool

use anyhow::Result;
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use actr_cli::core::{
    ActrCliError, Command, CommandContext, ConfigManager, ConsoleUI, ContainerBuilder,
    DefaultCacheManager, DefaultDependencyResolver, DefaultFingerprintValidator,
    DefaultNetworkValidator, DefaultProtoProcessor, DiscoveryContext, ErrorReporter,
    NetworkServiceDiscovery, ServiceContainer, TomlConfigManager,
};
use url::Url;

use actr_cli::commands::build as build_cmd;
use actr_cli::commands::deps as deps_cmd;
use actr_cli::commands::ops as ops_cmd;
use actr_cli::commands::pkg as pkg_cmd;
use actr_cli::commands::{
    CheckCommand, Command as LegacyCommand, ConfigCommand, DocCommand, GenCommand, InitCommand,
    InstallCommand, LogsCommand, PsCommand, RestartCommand, RmCommand, RunCommand, StartCommand,
    StopCommand,
};

/// ACTR-CLI - Actor-RTC Command Line Tool
#[derive(Parser)]
#[command(name = "actr")]
#[command(
    about = "Actor-RTC Command Line Tool",
    long_about = "Actor-RTC Command Line Tool - A unified CLI tool built on reuse architecture with 8 core components and 3 operation pipelines",
    version
)]
struct Cli {
    /// Verbosity level (use -vv for version and commit info)
    #[arg(short, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Actor project
    Init(InitCommand),

    /// Generate project documentation
    Doc(DocCommand),

    /// Generate code from proto files
    Gen(GenCommand),

    /// Validate project dependencies
    Check(CheckCommand),

    /// Manage project configuration
    Config(ConfigCommand),

    /// Install service dependencies declared in manifest.toml
    Install(InstallCommand),

    /// Build source artifact and package a signed .actr workload
    Build(build_cmd::BuildCommand),

    /// Run a packaged workload
    Run(RunCommand),

    /// Start a stopped detached runtime instance
    Start(StartCommand),

    /// List detached runtime instances
    Ps(PsCommand),

    /// Show logs for a detached runtime instance
    Logs(LogsCommand),

    /// Remove a detached runtime instance record
    Rm(RmCommand),

    /// Stop a detached runtime instance
    Stop(StopCommand),

    /// Restart a detached runtime instance
    Restart(RestartCommand),

    /// Package management (build, sign, verify, keygen)
    Pkg(pkg_cmd::PkgArgs),

    /// Dependency management (install, discover, fingerprint)
    Deps(deps_cmd::DepsArgs),

    /// Operations (dlq)
    Ops(ops_cmd::OpsArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_line_number(true)
        .with_file(true);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();

    let cli = Cli::parse();

    // Handle -vv for version and commit info
    if cli.verbose >= 2 {
        println!(
            "actr {} ({} {})",
            env!("CARGO_PKG_VERSION"),
            env!("ACTR_GIT_HASH"),
            env!("ACTR_GIT_DATE")
        );
        return Ok(());
    }

    // commands that do not need ServiceContainer; handle early
    if matches!(&cli.command, Some(Commands::Build(_))) {
        if let Some(Commands::Build(args)) = cli.command {
            return build_cmd::execute(args).await;
        }
    }

    if matches!(&cli.command, Some(Commands::Pkg(_))) {
        if let Some(Commands::Pkg(args)) = cli.command {
            return pkg_cmd::execute(args).await;
        }
    }

    // ops command does not need ServiceContainer; handle early
    if matches!(&cli.command, Some(Commands::Ops(_))) {
        if let Some(Commands::Ops(args)) = cli.command {
            return ops_cmd::execute(args).await;
        }
    }

    if matches!(
        &cli.command,
        Some(
            Commands::Run(_)
                | Commands::Start(_)
                | Commands::Restart(_)
                | Commands::Stop(_)
                | Commands::Ps(_)
                | Commands::Rm(_)
                | Commands::Logs(_)
        )
    ) {
        if let Some(cmd) = cli.command {
            return match cmd {
                Commands::Run(command) => command.execute().await.map_err(Into::into),
                Commands::Start(command) => command.execute().await.map_err(Into::into),
                Commands::Restart(command) => command.execute().await.map_err(Into::into),
                Commands::Stop(command) => command.execute().await.map_err(Into::into),
                Commands::Ps(command) => command.execute().await.map_err(Into::into),
                Commands::Rm(command) => command.execute().await.map_err(Into::into),
                Commands::Logs(command) => command.execute().await.map_err(Into::into),
                _ => unreachable!(),
            };
        }
    }

    // Build service container for remaining commands
    let container = build_container(None).await?;

    let context = CommandContext {
        container: Arc::new(std::sync::Mutex::new(container)),
        args: actr_cli::core::CommandArgs {
            command: String::new(),
            subcommand: None,
            flags: std::collections::HashMap::new(),
            positional: Vec::new(),
        },
        working_dir: std::env::current_dir()?,
    };

    if let Some(cmd) = &cli.command {
        match execute_command(cmd, &context).await {
            Ok(result) => match result {
                actr_cli::core::CommandResult::Success(msg) => {
                    if msg != "Help displayed" {
                        println!("{msg}");
                    }
                }
                actr_cli::core::CommandResult::Install(install_result) => {
                    println!("Installation complete: {}", install_result.summary());
                }
                actr_cli::core::CommandResult::Validation(validation_report) => {
                    let formatted = ErrorReporter::format_validation_report(&validation_report);
                    println!("{formatted}");
                }
                actr_cli::core::CommandResult::Generation(gen_result) => {
                    println!("Generated {} files", gen_result.generated_files.len());
                }
                actr_cli::core::CommandResult::Error(error) => {
                    eprintln!("{} {error}", "Error:".red());
                    std::process::exit(1);
                }
            },
            Err(e) => {
                if let Some(cli_error) = e.downcast_ref::<ActrCliError>() {
                    if matches!(cli_error, ActrCliError::OperationCancelled) {
                        std::process::exit(0);
                    }
                    eprintln!("{}", ErrorReporter::format_error(cli_error));
                } else {
                    eprintln!("{} {e:?}", "Error:".red());
                }
                std::process::exit(1);
            }
        }
    } else {
        use clap::CommandFactory;
        Cli::command().print_help()?;
    }

    Ok(())
}

async fn build_container(key_override: Option<&str>) -> Result<ServiceContainer> {
    let config_path = std::path::Path::new("manifest.toml");
    let mut builder = ContainerBuilder::new();
    let mut config_manager = None;

    if config_path.exists() {
        builder = builder.config_path(config_path);
    }

    let mut container = builder.build()?;

    container = container.register_user_interface(Arc::new(ConsoleUI::new()));

    if config_path.exists() {
        let manager = Arc::new(TomlConfigManager::new(config_path));
        container = container.register_config_manager(manager.clone());
        config_manager = Some(manager);
    }

    let mut container =
        container.register_dependency_resolver(Arc::new(DefaultDependencyResolver::new()));

    container = container.register_network_validator(Arc::new(DefaultNetworkValidator::new()));
    container =
        container.register_fingerprint_validator(Arc::new(DefaultFingerprintValidator::new()));
    container = container.register_proto_processor(Arc::new(DefaultProtoProcessor::new()));
    container = container.register_cache_manager(Arc::new(DefaultCacheManager::new()));

    if let Some(manager) = config_manager {
        let config = manager.load_config(config_path).await?;
        let effective_cli =
            actr_cli::config::resolver::resolve_effective_cli_config().unwrap_or_default();

        let signaling_url = Url::parse(&effective_cli.network.signaling_url).map_err(|e| {
            anyhow::anyhow!(
                "Invalid network.signaling_url '{}': {}",
                effective_cli.network.signaling_url,
                e
            )
        })?;

        let ais_endpoint = effective_cli.network.ais_endpoint.clone();

        let realm_id = effective_cli.network.realm_id.unwrap_or(1);

        let realm_secret = effective_cli.network.realm_secret.clone();

        // Read manifest.toml raw bytes and try to sign for AIS Path 2 identity verification.
        // This allows `actr install` to register with AIS without a published package.
        // AIS Path 2 requires `signing_key_id` in the manifest, so we inject it if missing.
        let (manifest_raw, mfr_signature) = {
            if config_path.exists() {
                // Try to load signing key — check multiple locations
                let try_load_key = |p: &std::path::Path| -> Option<ed25519_dalek::SigningKey> {
                    let json_str = std::fs::read_to_string(p).ok()?;
                    let json: serde_json::Value = serde_json::from_str(&json_str).ok()?;
                    let priv_b64 = json["private_key"].as_str()?;
                    let priv_bytes = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        priv_b64,
                    )
                    .ok()?;
                    let arr: [u8; 32] = priv_bytes.try_into().ok()?;
                    Some(ed25519_dalek::SigningKey::from_bytes(&arr))
                };

                // Resolve keychain path: only use configured path from config.toml ([mfr].keychain)
                let configured_key_path =
                    key_override.map(std::path::PathBuf::from).or_else(|| {
                        effective_cli.mfr.keychain.as_deref().map(|kc_path| {
                            if let Some(stripped) = kc_path.strip_prefix("~/") {
                                dirs::home_dir()
                                    .map(|h| h.join(stripped))
                                    .unwrap_or_else(|| std::path::PathBuf::from(kc_path))
                            } else {
                                std::path::PathBuf::from(kc_path)
                            }
                        })
                    });

                let signing_key = configured_key_path.as_deref().and_then(try_load_key);

                match signing_key {
                    Some(signing_key) => {
                        use ed25519_dalek::Signer;
                        let key_id =
                            actr_pack::compute_key_id(&signing_key.verifying_key().to_bytes());

                        // Build a flat manifest for AIS Path 2 identity verification.
                        // AIS verify_mfr_identity expects manufacturer/name/version/signing_key_id
                        // at the TOML top level, but manifest.toml from `actr init` nests them
                        // inside [package]. Construct a canonical flat TOML with the required fields.
                        let actr_type = &config.package.actr_type;
                        let manifest_bytes = format!(
                            "manufacturer = \"{}\"\nname = \"{}\"\nversion = \"{}\"\nsigning_key_id = \"{}\"\n",
                            actr_type.manufacturer, actr_type.name, actr_type.version, key_id
                        )
                        .into_bytes();

                        let signature = signing_key.sign(&manifest_bytes).to_bytes().to_vec();
                        (Some(manifest_bytes), Some(signature))
                    }
                    // Keep local-only commands like `actr gen` and `actr install`
                    // usable without a configured signing key. When a keychain is
                    // configured explicitly but cannot be read, fail fast because
                    // the caller opted into AIS Path 2 signing.
                    None => {
                        if let Some(path) = configured_key_path {
                            anyhow::bail!("Failed to load signing key from {}", path.display());
                        }
                        (None, None)
                    }
                }
            } else {
                (None, None)
            }
        };

        let discovery_context = DiscoveryContext {
            package_actr_type: config.package.actr_type.clone(),
            signaling_url,
            ais_endpoint,
            realm: actr_protocol::Realm { realm_id },
            realm_secret,
            manifest_raw,
            mfr_signature,
        };

        container = container
            .register_service_discovery(Arc::new(NetworkServiceDiscovery::new(discovery_context)));
    }
    Ok(container)
}

async fn execute_command(
    command: &Commands,
    context: &CommandContext,
) -> Result<actr_cli::core::CommandResult> {
    match command {
        Commands::Init(cmd) => match cmd.execute().await {
            Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                "Project initialized".to_string(),
            )),
            Err(e) => Err(e.into()),
        },
        Commands::Doc(cmd) => match cmd.execute().await {
            Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                "Documentation generated".to_string(),
            )),
            Err(e) => Err(e.into()),
        },
        Commands::Check(cmd) => {
            if cmd.config_file.is_none() {
                let container = context.container.lock().unwrap();
                container.validate(&cmd.required_components())?;
            }
            cmd.execute(context).await
        }
        Commands::Gen(cmd) => match cmd.execute().await {
            Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                "Generation completed".to_string(),
            )),
            Err(e) => Err(e.into()),
        },
        Commands::Config(cmd) => {
            use actr_cli::core::Command;
            cmd.execute(context).await
        }
        Commands::Install(cmd) => {
            let command = InstallCommand::from_args(cmd);
            {
                let container = context.container.lock().unwrap();
                container.validate(&command.required_components())?;
            }
            command.execute(context).await
        }
        Commands::Build(_) => unreachable!("build is handled before build_container"),
        Commands::Run(_)
        | Commands::Start(_)
        | Commands::Restart(_)
        | Commands::Stop(_)
        | Commands::Ps(_)
        | Commands::Rm(_)
        | Commands::Logs(_) => {
            unreachable!("runtime commands are handled before build_container")
        }
        Commands::Deps(args) => deps_cmd::execute_with_context(args, context).await,
        Commands::Pkg(_) => unreachable!("pkg is handled before build_container"),
        Commands::Ops(_) => unreachable!("ops is handled before build_container"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_build_container() {
        let container = build_container(None).await;
        assert!(container.is_ok());
    }
}
