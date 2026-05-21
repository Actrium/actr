//! `actr version` — print version, git hash, and build date.

use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use anyhow::Result;
use async_trait::async_trait;
use clap::Args;

#[derive(Args, Debug)]
#[command(
    about = "Print version, git hash, and build date",
    long_about = "Print the actr CLI version, git commit hash, and build date."
)]
pub struct VersionCommand {
    /// Emit machine-readable JSON instead of human text
    #[arg(long)]
    pub json: bool,
}

#[async_trait]
impl Command for VersionCommand {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        let version = env!("CARGO_PKG_VERSION");
        let hash = env!("ACTR_GIT_HASH");
        let date = env!("ACTR_GIT_DATE");
        if self.json {
            println!(
                "{}",
                serde_json::json!({
                    "version": version,
                    "git_hash": hash,
                    "git_date": date,
                })
            );
        } else {
            println!("actr {} ({} {})", version, hash, date);
        }
        Ok(CommandResult::Success(String::new()))
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "version"
    }

    fn description(&self) -> &str {
        "Print version, git hash, and build date"
    }
}
