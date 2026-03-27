//! `actr deps` — Dependency management commands
//!
//! Groups install, discover, and fingerprint sub-commands.

use anyhow::Result;
use clap::{Args, Subcommand};

use super::discovery::DiscoveryCommand;
use super::fingerprint::FingerprintCommand;
use crate::core::{CommandContext, CommandResult};

#[derive(Args, Debug)]
pub struct DepsArgs {
    #[command(subcommand)]
    pub command: DepsCommand,
}

#[derive(Subcommand, Debug)]
pub enum DepsCommand {
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
        DepsCommand::Discover(cmd) => {
            if !std::path::Path::new("manifest.toml").exists() {
                return Err(anyhow::anyhow!(
                    "No manifest.toml found in current directory.\n\u{1f4a1} Hint: Run 'actr init' to initialize a new workload project first."
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
