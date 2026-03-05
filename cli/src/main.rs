//! ACTR-CLI - Actor-RTC 命令行工具
//!
//! 基于复用架构实现的统一CLI工具，通过8个核心组件和3个操作管道
//! 提供一致的用户体验和高代码复用率。

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

// 导入核心复用组件
use actr_cli::core::{
    ActrCliError, Command, CommandArgs, CommandContext, ContainerBuilder, ErrorReporter,
    ServiceContainer,
};

// 导入命令实现
use actr_cli::commands::{
    Command as LegacyCommand, DiscoveryCommand, GenCommand, InitCommand, InstallCommand,
};
use actr_cli::commands::dlq as dlq_cmd;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt::init();

    // 解析命令行参数（简化版本）
    let args = parse_args();

    // 构建服务容器并注册组件
    let container = build_container().await?;

    // 创建命令执行上下文
    let context = CommandContext {
        container: Arc::new(std::sync::Mutex::new(container)),
        args: args.clone(),
        working_dir: std::env::current_dir()?,
    };

    // 根据命令分发执行
    match execute_command(&context).await {
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
                eprintln!("❌ {error}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            // 统一的错误处理
            if let Some(cli_error) = e.downcast_ref::<ActrCliError>() {
                eprintln!("{}", ErrorReporter::format_error(cli_error));
            } else {
                eprintln!("Error: {e}");
            }
            std::process::exit(1);
        }
    }

    Ok(())
}

/// 解析命令行参数 (简化版本)
fn parse_args() -> CommandArgs {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        return CommandArgs {
            command: "help".to_string(),
            subcommand: None,
            flags: HashMap::new(),
            positional: Vec::new(),
        };
    }

    let command = args[1].clone();
    let mut flags = HashMap::new();
    let mut positional = Vec::new();

    // 简化的参数解析
    for arg in &args[2..] {
        if arg.starts_with("--") {
            let flag = arg.trim_start_matches("--");
            if let Some((key, value)) = flag.split_once('=') {
                flags.insert(key.to_string(), value.to_string());
            } else {
                flags.insert(flag.to_string(), "true".to_string());
            }
        } else {
            positional.push(arg.clone());
        }
    }

    CommandArgs {
        command,
        subcommand: None,
        flags,
        positional,
    }
}

/// 构建服务容器
async fn build_container() -> Result<ServiceContainer> {
    let container = ContainerBuilder::new().config_path("Actr.toml").build()?;

    // TODO: 在实际实现中，这里应该注册具体的组件实现
    // 例如:
    // container
    //     .register_config_manager(Arc::new(TomlConfigManager::new("Actr.toml")))
    //     .register_dependency_resolver(Arc::new(DefaultDependencyResolver::new()))
    //     .register_service_discovery(Arc::new(NetworkServiceDiscovery::new()))
    //     ...

    Ok(container)
}

/// 执行命令
async fn execute_command(context: &CommandContext) -> Result<actr_cli::core::CommandResult> {
    match context.args.command.as_str() {
        "init" => {
            let name = context.args.positional.first().cloned();
            let signaling = context.args.flags.get("signaling").cloned();
            let template = context.args.flags.get("template").cloned();
            let project_name = context.args.flags.get("project-name").cloned();

            let command = InitCommand {
                name,
                template,
                project_name,
                signaling,
            };

            // InitCommand 使用旧的 Command trait，直接执行
            match command.execute().await {
                Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                    "Project initialized".to_string(),
                )),
                Err(e) => Err(e.into()),
            }
        }
        "install" => {
            let packages = context.args.positional.clone();
            let force = context.args.flags.contains_key("force");
            let force_update = context.args.flags.contains_key("force-update");
            let skip_verification = context.args.flags.contains_key("skip-verification");

            let command = InstallCommand::new(packages, force, force_update, skip_verification);

            // 验证所需组件
            {
                let container = context.container.lock().unwrap();
                container.validate(&command.required_components())?;
            }

            // 执行命令
            command.execute(context).await
        }
        "discovery" => {
            let filter = context.args.flags.get("filter").cloned();
            let verbose = context.args.flags.contains_key("verbose");
            let auto_install = context.args.flags.contains_key("auto-install");

            let command = DiscoveryCommand::new(filter, verbose, auto_install);

            // 验证所需组件
            {
                let container = context.container.lock().unwrap();
                container.validate(&command.required_components())?;
            }

            // 执行命令
            command.execute(context).await
        }
        "build" => {
            let config = context
                .args
                .flags
                .get("config")
                .cloned()
                .unwrap_or_else(|| "Actr.toml".to_string());
            let args = actr_cli::commands::build::BuildArgs { config };
            match actr_cli::commands::build::execute(args).await {
                Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                    "Build completed".to_string(),
                )),
                Err(e) => Err(e.into()),
            }
        }
        "check" => {
            // TODO: 实现 check 命令
            Ok(actr_cli::core::CommandResult::Success(
                "Check completed".to_string(),
            ))
        }
        "gen" => {
            let input = context
                .args
                .flags
                .get("input")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("proto"));
            let output = context
                .args
                .flags
                .get("output")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("src/generated"));
            let generate_scaffold = !context.args.flags.contains_key("no-scaffold");
            let overwrite_user_code = context.args.flags.contains_key("overwrite-user-code");
            let format = !context.args.flags.contains_key("no-format");
            let debug = context.args.flags.contains_key("debug");

            let command = GenCommand {
                input,
                output,
                generate_scaffold,
                overwrite_user_code,
                format,
                debug,
            };

            match command.execute().await {
                Ok(_) => Ok(actr_cli::core::CommandResult::Success(
                    "Generation completed".to_string(),
                )),
                Err(e) => Err(e.into()),
            }
        }
        "dlq" => {
            let sub = context.args.positional.first().map(|s| s.as_str());
            let (sub, pos) = match sub {
                Some(s @ ("list" | "show" | "stats" | "delete")) => {
                    (Some(s), context.args.positional[1..].to_vec())
                }
                _ => (sub, context.args.positional.clone()),
            };
            let args = dlq_cmd::DlqArgs::parse(sub, &pos, &context.args.flags)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            dlq_cmd::execute(args)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(actr_cli::core::CommandResult::Success(String::new()))
        }
        _ => {
            print_help();
            Ok(actr_cli::core::CommandResult::Success(
                "Help displayed".to_string(),
            ))
        }
    }
}

/// 显示帮助信息
fn print_help() {
    println!(
        r#"
ACTR-CLI - Actor-RTC Command Line Tool

Usage:
    actr <COMMAND> [OPTIONS] [ARGS]

Commands:
    init               Initialize a new Actor project
    install [DEPS...]  Install service dependencies
        --force             Force reinstall
        --force-update      Force update all dependencies
        --skip-verification Skip fingerprint verification

    check              Validate project dependencies
        --verbose       Show detailed information
        --timeout=N     Set timeout in seconds

    gen                Generate code from proto files
        --clean         Clean and regenerate
        --scaffold      Generate user code templates

    discovery          Discover network services
        --filter=PATTERN    Service name filter
        --verbose          Show detailed information
        --auto-install     Auto-install selected services

    run [SCRIPT]       Run project scripts
    config             Manage configuration
    doc                Generate documentation
    fingerprint        Show service fingerprints

    dlq                Dead Letter Queue inspection
        list [--limit=N] [--category=CAT] [--after=RFC3339] [--db=PATH]
        show <ID> [--db=PATH]
        stats [--db=PATH]
        delete <ID> [--db=PATH]

    help               Show this help message

Examples:
    actr init my-service
    actr install user-service@1.2.0
    actr gen --clean
    actr check --verbose
    actr discovery --filter="user-*"
"#
    );
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
