//! # actr-cli
//! 
//! Command line tool for Actor-RTC framework projects.
//! 
//! This tool provides commands for initializing, building, and running Actor-RTC projects.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod error;
mod templates;
mod utils;

use commands::{build::BuildCommand, init::InitCommand, run::RunCommand, Command};

#[derive(Parser)]
#[command(name = "actr-cli")]
#[command(about = "Command line tool for Actor-RTC framework projects")]
#[command(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress output (except errors)
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new Actor-RTC project
    Init(InitCommand),

    /// Build the project and generate code
    Build(BuildCommand),

    /// Run the project
    Run(RunCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    if let Err(e) = init_logging(cli.verbose, cli.quiet) {
        eprintln!("Failed to initialize logging: {}", e);
        return Err(anyhow::anyhow!("Logging initialization failed"));
    }

    // Execute the command
    match cli.command {
        Commands::Init(cmd) => cmd.execute().await.map_err(|e| anyhow::anyhow!(e)),
        Commands::Build(cmd) => cmd.execute().await.map_err(|e| anyhow::anyhow!(e)),
        Commands::Run(cmd) => cmd.execute().await.map_err(|e| anyhow::anyhow!(e)),
    }
}

fn init_logging(verbose: bool, quiet: bool) -> Result<()> {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = if verbose {
        "debug"
    } else if quiet {
        "warn"
    } else {
        "info"
    };

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(filter));

    fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .compact()
        .init();

    Ok(())
}