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
    DefaultNetworkValidator, DefaultProtoProcessor, ErrorReporter, NetworkServiceDiscovery,
    ServiceContainer, TomlConfigManager,
};

use actr_cli::commands::deps as deps_cmd;
use actr_cli::commands::ops as ops_cmd;
use actr_cli::commands::pkg as pkg_cmd;
use actr_cli::commands::{
    CheckCommand, Command as LegacyCommand, ConfigCommand, DocCommand, GenCommand, InitCommand,
    InstallCommand, RunCommand,
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

    /// Run project scripts
    Run(RunCommand),

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

    // pkg command does not need ServiceContainer; handle early
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

    // Build service container for remaining commands
    let container = build_container().await?;

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
        container =
            container.register_service_discovery(Arc::new(NetworkServiceDiscovery::new(config)));
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
        Commands::Run(cmd) => match cmd.execute().await {
            Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                "Script executed".to_string(),
            )),
            Err(e) => Err(e.into()),
        },
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
        let container = build_container().await;
        assert!(container.is_ok());
    }
}
