//! `actr deps` — local dependency management.
//!
//! Subcommands:
//!   - `install` — install service dependencies declared in manifest.toml,
//!     or add a new one (`actr deps install <alias> --actr-type ...`).

use anyhow::Result;
use async_trait::async_trait;
use clap::{Args, Subcommand};

use super::install::InstallCommand;
use crate::core::{Command, CommandContext, CommandResult, ComponentType};

#[derive(Args, Debug)]
pub struct DepsArgs {
    #[command(subcommand)]
    pub command: DepsCommand,
}

#[derive(Subcommand, Debug)]
pub enum DepsCommand {
    /// Install service dependencies (all from manifest.toml, or a specific one).
    Install(InstallCommand),
}

#[async_trait]
impl Command for DepsArgs {
    async fn execute(&self, ctx: &CommandContext) -> Result<CommandResult> {
        match &self.command {
            DepsCommand::Install(cmd) => {
                let command = InstallCommand::from_args(cmd);
                {
                    let container = ctx.container.lock().unwrap();
                    container.validate(&command.required_components())?;
                }
                command.execute(ctx).await
            }
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        match &self.command {
            DepsCommand::Install(cmd) => cmd.required_components(),
        }
    }

    fn name(&self) -> &str {
        "deps"
    }

    fn description(&self) -> &str {
        "Manage local service dependencies"
    }
}
