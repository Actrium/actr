//! `actr deps` — Dependency management commands
//!
//! Groups install, discover, and fingerprint sub-commands.

use anyhow::Result;
use clap::{Args, Subcommand};

use super::discovery::DiscoveryCommand;
use super::fingerprint::FingerprintCommand;
use super::install::InstallCommand;
use crate::core::{CommandContext, CommandResult};

#[derive(Args, Debug)]
pub struct DepsArgs {
    #[command(subcommand)]
    pub command: DepsCommand,
}

#[derive(Subcommand, Debug)]
pub enum DepsCommand {
    /// Install service dependencies
    Install(InstallCommand),
    /// Discover network services
    Discover(DiscoveryCommand),
    /// Compute semantic fingerprints
    Fingerprint(FingerprintCommand),
}

pub async fn execute_with_context(
    args: &DepsArgs,
    context: &CommandContext,
) -> Result<CommandResult> {
    use crate::core::Command;
    match &args.command {
        DepsCommand::Install(cmd) => {
            let command = InstallCommand::from_args(cmd);
            {
                let container = context.container.lock().unwrap();
                container.validate(&command.required_components())?;
            }
            command.execute(context).await
        }
        DepsCommand::Discover(cmd) => {
            if !std::path::Path::new("actr.toml").exists() {
                return Err(anyhow::anyhow!(
                    "No actr.toml found in current directory.\n\u{1f4a1} Hint: Run 'actr init' to initialize a new project first."
                ));
            }
            let command = DiscoveryCommand::from_args(cmd);
            {
                let container = context.container.lock().unwrap();
                container.validate(&command.required_components())?;
            }
            command.execute(context).await
        }
        DepsCommand::Fingerprint(cmd) => cmd.execute(context).await,
    }
}
