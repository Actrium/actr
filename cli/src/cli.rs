//! CLI surface: `Cli`/`Commands` enum + unified dispatch.
//!
//! Guiding principles (see `cli/README.md` for the long form):
//!
//! - **High-frequency commands are flat**: `init`, `gen`, `build`, `run`,
//!   `ps`, `logs`, `start/stop/restart/rm`, `check`, `doc`.
//! - **Low-frequency / fine-grained operations are grouped**:
//!   `deps`, `pkg`, `registry`, `dlq`.
//! - **Meta commands** sit at the top level: `config`, `version`, `completion`.
//! - **Every subcommand implements [`crate::core::Command`]**; main dispatches
//!   through a single `cmd.execute(&ctx)` call and only builds a
//!   `ServiceContainer` when `cmd.required_components()` is non-empty.

use std::sync::Arc;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use owo_colors::OwoColorize;
use url::Url;

use crate::commands::{
    BuildCommand, CheckCommand, CompletionCommand, ConfigCommand, DepsArgs, DlqArgs, DocCommand,
    GenCommand, InitCommand, LogsCommand, PkgArgs, PsCommand, RegistryArgs, RestartCommand,
    RmCommand, RunCommand, StartCommand, StopCommand, VersionCommand,
};
use crate::core::{
    ActrCliError, Command, CommandContext, CommandResult, ConfigManager, ConsoleUI,
    ContainerBuilder, DefaultCacheManager, DefaultDependencyResolver, DefaultFingerprintValidator,
    DefaultNetworkValidator, DefaultProtoProcessor, DiscoveryContext, ErrorReporter,
    NetworkServiceDiscovery, ServiceContainer, TomlConfigManager,
};

/// Top-level `actr` CLI.
#[derive(Parser)]
#[command(name = "actr")]
#[command(
    about = "Actor-RTC Command Line Tool",
    long_about = "Actor-RTC Command Line Tool.\n\n\
        Commands are grouped by audience:\n  \
        development:  init / gen / build / check / doc\n  \
        runtime:      run / ps / logs / start / stop / restart / rm\n  \
        resources:    deps / pkg / registry / dlq\n  \
        meta:         config / version / completion",
    version,
    disable_version_flag = true
)]
pub struct Cli {
    /// Verbosity level (currently unused; -v is reserved for future telemetry).
    #[arg(short, action = clap::ArgAction::Count, hide = true)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    // ── development (flat, high-frequency) ──
    /// Initialize a new Actor project
    Init(InitCommand),
    /// Generate code from proto files
    Gen(GenCommand),
    /// Build source artifact and package a signed .actr workload
    Build(BuildCommand),
    /// Validate project dependencies
    Check(CheckCommand),
    /// Generate project documentation
    Doc(DocCommand),

    // ── runtime (flat, docker-style) ──
    /// Run a packaged workload
    Run(RunCommand),
    /// List detached runtime instances
    Ps(PsCommand),
    /// Show logs for a detached runtime instance
    Logs(LogsCommand),
    /// Start a stopped detached runtime instance
    Start(StartCommand),
    /// Stop a detached runtime instance
    Stop(StopCommand),
    /// Restart a detached runtime instance
    Restart(RestartCommand),
    /// Remove a detached runtime instance record
    Rm(RmCommand),

    // ── resources (grouped) ──
    /// Local dependency management (install)
    Deps(DepsArgs),
    /// Local package operations (sign, verify, keygen)
    Pkg(PkgArgs),
    /// Remote service registry (discover, publish, fingerprint)
    Registry(RegistryArgs),
    /// Dead Letter Queue inspection and remediation
    Dlq(DlqArgs),

    // ── meta ──
    /// Manage project configuration
    Config(ConfigCommand),
    /// Print version, git hash, and build date
    Version(VersionCommand),
    /// Generate shell completion script
    Completion(CompletionCommand),
}

impl Commands {
    /// Cast the parsed subcommand to its [`Command`] trait object.
    pub fn as_command(&self) -> &dyn Command {
        match self {
            Commands::Init(c) => c,
            Commands::Gen(c) => c,
            Commands::Build(c) => c,
            Commands::Check(c) => c,
            Commands::Doc(c) => c,
            Commands::Run(c) => c,
            Commands::Ps(c) => c,
            Commands::Logs(c) => c,
            Commands::Start(c) => c,
            Commands::Stop(c) => c,
            Commands::Restart(c) => c,
            Commands::Rm(c) => c,
            Commands::Deps(c) => c,
            Commands::Pkg(c) => c,
            Commands::Registry(c) => c,
            Commands::Dlq(c) => c,
            Commands::Config(c) => c,
            Commands::Version(c) => c,
            Commands::Completion(c) => c,
        }
    }
}

/// Build the raw clap [`clap::Command`] for completion-script generation.
pub fn build_cli() -> clap::Command {
    Cli::command()
}

/// Entry point for `main.rs`.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    let Some(cmd) = cli.command else {
        Cli::command().print_help()?;
        return Ok(());
    };

    let command = cmd.as_command();
    let needs_container = !command.required_components().is_empty();
    let container = if needs_container {
        build_container().await?
    } else {
        ContainerBuilder::new().build()?
    };

    let ctx = CommandContext {
        container: Arc::new(std::sync::Mutex::new(container)),
        args: crate::core::CommandArgs {
            command: String::new(),
            subcommand: None,
            flags: std::collections::HashMap::new(),
            positional: Vec::new(),
        },
        working_dir: std::env::current_dir()?,
    };

    match command.execute(&ctx).await {
        Ok(result) => {
            render_result(result);
            Ok(())
        }
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
}

fn render_result(result: CommandResult) {
    match result {
        CommandResult::Success(msg) => {
            if !msg.is_empty() && msg != "Help displayed" {
                println!("{msg}");
            }
        }
        CommandResult::Install(install_result) => {
            println!("Installation complete: {}", install_result.summary());
        }
        CommandResult::Validation(report) => {
            let formatted = ErrorReporter::format_validation_report(&report);
            println!("{formatted}");
        }
        CommandResult::Generation(gen_result) => {
            println!("Generated {} files", gen_result.generated_files.len());
        }
        CommandResult::Error(error) => {
            eprintln!("{} {error}", "Error:".red());
            std::process::exit(1);
        }
    }
}

async fn build_container() -> Result<ServiceContainer> {
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
            crate::config::resolver::resolve_effective_cli_config().unwrap_or_default();

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

        // Attempt to sign manifest.toml for AIS Path 2 identity registration.
        // Without a signing key we still allow local-only commands (gen, doc, check).
        let (manifest_raw, mfr_signature) = build_mfr_identity(&effective_cli, &config)?;

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

type MfrIdentity = (Option<Vec<u8>>, Option<Vec<u8>>);

fn build_mfr_identity(
    effective_cli: &crate::config::resolver::EffectiveCliConfig,
    config: &actr_config::ManifestConfig,
) -> Result<MfrIdentity> {
    let configured_key_path = effective_cli.mfr.keychain.as_deref().map(|kc_path| {
        if let Some(stripped) = kc_path.strip_prefix("~/") {
            dirs::home_dir()
                .map(|h| h.join(stripped))
                .unwrap_or_else(|| std::path::PathBuf::from(kc_path))
        } else {
            std::path::PathBuf::from(kc_path)
        }
    });

    let try_load_key = |p: &std::path::Path| -> Option<ed25519_dalek::SigningKey> {
        let json_str = std::fs::read_to_string(p).ok()?;
        let json: serde_json::Value = serde_json::from_str(&json_str).ok()?;
        let priv_b64 = json["private_key"].as_str()?;
        let priv_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, priv_b64).ok()?;
        let arr: [u8; 32] = priv_bytes.try_into().ok()?;
        Some(ed25519_dalek::SigningKey::from_bytes(&arr))
    };

    let signing_key = configured_key_path.as_deref().and_then(try_load_key);

    match signing_key {
        Some(signing_key) => {
            use ed25519_dalek::Signer;
            let key_id = actr_pack::compute_key_id(&signing_key.verifying_key().to_bytes());
            let actr_type = &config.package.actr_type;
            let manifest_bytes = format!(
                "manufacturer = \"{}\"\nname = \"{}\"\nversion = \"{}\"\nsigning_key_id = \"{}\"\n",
                actr_type.manufacturer, actr_type.name, actr_type.version, key_id
            )
            .into_bytes();
            let signature = signing_key.sign(&manifest_bytes).to_bytes().to_vec();
            Ok((Some(manifest_bytes), Some(signature)))
        }
        None => {
            if let Some(path) = configured_key_path {
                anyhow::bail!("Failed to load signing key from {}", path.display());
            }
            Ok((None, None))
        }
    }
}
