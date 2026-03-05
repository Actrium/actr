//! ACTR-CLI - Actor-RTC 命令行工具
//!
//! 基于复用架构实现的统一CLI工具，通过8个核心组件和3个操作管道
//! 提供一致的用户体验和高代码复用率。

use anyhow::Result;
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// 导入核心复用组件
use actr_cli::core::{
    ActrCliError, Command, CommandContext, ConfigManager, ConsoleUI, ContainerBuilder,
    DefaultCacheManager, DefaultDependencyResolver, DefaultFingerprintValidator,
    DefaultNetworkValidator, DefaultProtoProcessor, ErrorReporter, NetworkServiceDiscovery,
    ServiceContainer, TomlConfigManager,
};

// 导入命令实现
use actr_cli::commands::{
    CheckCommand, Command as LegacyCommand, ConfigCommand, DiscoveryCommand, DocCommand,
    FingerprintCommand, GenCommand, InitCommand, InstallCommand, RunCommand,
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

    /// Install service dependencies
    Install(InstallCommand),

    /// Discover network services
    Discovery(DiscoveryCommand),

    /// Generate project documentation
    Doc(DocCommand),

    /// Generate code from proto files
    Gen(GenCommand),

    /// Validate project dependencies
    Check(CheckCommand),

    /// Compute semantic fingerprints
    Fingerprint(FingerprintCommand),

    /// Manage project configuration
    Config(ConfigCommand),

    /// Run project scripts
    Run(RunCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
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

    // 使用 clap 解析命令行参数
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

    // 构建服务容器并注册组件
    let container = build_container().await?;

    // 创建命令执行上下文
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

    // 根据命令分发执行
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
                    eprintln!("{} {error}", "❌".red());
                    std::process::exit(1);
                }
            },
            Err(e) => {
                // 统一的错误处理
                if let Some(cli_error) = e.downcast_ref::<ActrCliError>() {
                    if matches!(cli_error, ActrCliError::OperationCancelled) {
                        // Exit silently
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

/// 构建服务容器
async fn build_container() -> Result<ServiceContainer> {
    let config_path = std::path::Path::new("Actr.toml");
    let mut builder = ContainerBuilder::new();
    let mut config_manager = None;

    if config_path.exists() {
        builder = builder.config_path(config_path);
    }

    let mut container = builder.build()?;

    // Register UI component (always available)
    container = container.register_user_interface(Arc::new(ConsoleUI::new()));

    if config_path.exists() {
        let manager = Arc::new(TomlConfigManager::new(config_path));
        container = container.register_config_manager(manager.clone());
        config_manager = Some(manager);
    }

    let mut container =
        container.register_dependency_resolver(Arc::new(DefaultDependencyResolver::new()));

    // Register network validator (stub implementation)
    container = container.register_network_validator(Arc::new(DefaultNetworkValidator::new()));

    // Register fingerprint validator
    container =
        container.register_fingerprint_validator(Arc::new(DefaultFingerprintValidator::new()));

    // Register proto processor
    container = container.register_proto_processor(Arc::new(DefaultProtoProcessor::new()));

    // Register cache manager
    container = container.register_cache_manager(Arc::new(DefaultCacheManager::new()));

    if let Some(manager) = config_manager {
        let config = manager.load_config(config_path).await?;
        container =
            container.register_service_discovery(Arc::new(NetworkServiceDiscovery::new(config)));
    }
    Ok(container)
}

/// 执行命令
async fn execute_command(
    command: &Commands,
    context: &CommandContext,
) -> Result<actr_cli::core::CommandResult> {
    match command {
        Commands::Init(cmd) => {
            // InitCommand 使用旧的 Command trait，直接执行
            match cmd.execute().await {
                Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                    "Project initialized".to_string(),
                )),
                Err(e) => Err(e.into()),
            }
        }
        Commands::Install(cmd) => {
            let command = InstallCommand::from_args(cmd);

            // 验证所需组件
            context
                .container
                .lock()
                .unwrap()
                .validate(&command.required_components())?;

            // 执行命令
            command.execute(context).await
        }
        Commands::Discovery(cmd) => {
            let command = DiscoveryCommand::from_args(cmd);

            // TODO: (Option B) In the future, if a default public signaling server is available,
            // we should allow discovery to run without a local Actr.toml by using a default config.
            // For now (Option A), we require a project context to get the signaling URL.
            if !std::path::Path::new("Actr.toml").exists() {
                return Err(anyhow::anyhow!(
                    "No Actr.toml found in current directory.\n💡 Hint: Run 'actr init' to initialize a new project first."
                ));
            }

            // 验证所需组件
            {
                let container = context.container.lock().unwrap();
                container.validate(&command.required_components())?;
            }

            // 执行命令
            command.execute(context).await
        }
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
        Commands::Fingerprint(cmd) => cmd.execute(context).await,
        Commands::Gen(cmd) => match cmd.execute().await {
            Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                "Generation completed".to_string(),
            )),
            Err(e) => Err(e.into()),
        },
        Commands::Config(cmd) => {
            // Config command uses the new Command trait from core
            use actr_cli::core::Command;
            cmd.execute(context).await
        }
        Commands::Run(cmd) => match cmd.execute().await {
            Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                "Script executed".to_string(),
            )),
            Err(e) => Err(e.into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_simple() {
        // SAFETY: This is safe in a single-threaded test environment
        // We're only setting an env var that won't be used by other threads
        unsafe {
            std::env::set_var("args", "actr install user-service");
        }
        // TODO: 实现参数解析测试
    }

    #[tokio::test]
    async fn test_build_container() {
        let container = build_container().await;
        assert!(container.is_ok());
    }
}
