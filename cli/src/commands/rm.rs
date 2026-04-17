use crate::commands::process::{kill_process, wait_for_exit};
use crate::commands::runtime_state::{RuntimeStateStore, RuntimeStatus, resolve_hyper_dir};
use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use crate::error::ActrCliError;
use anyhow::Result;
use async_trait::async_trait;
use clap::Args;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args, Debug)]
pub struct RmCommand {
    /// WID or WID prefix (min 8 chars)
    #[arg(value_name = "WID")]
    pub wid: String,

    /// Runtime configuration file
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Hyper data directory
    #[arg(long = "hyper-dir", value_name = "DIR")]
    pub hyper_dir: Option<PathBuf>,

    /// Force remove even if workload is running
    #[arg(short = 'f', long = "force")]
    pub force: bool,
}

#[async_trait]
impl Command for RmCommand {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let entry = store.resolve_wid_prefix(&self.wid).await?;

        if entry.status == RuntimeStatus::Running {
            if !self.force {
                return Err(ActrCliError::command_error(format!(
                    "Workload {} is running. Stop it first or use -f to force remove.",
                    entry.wid_short()
                ))
                .into());
            }

            if kill_process(entry.record.pid)?
                && !wait_for_exit(entry.record.pid, Duration::from_secs(1)).await
            {
                return Err(ActrCliError::command_error(format!(
                    "Process {} did not exit after SIGKILL",
                    entry.record.pid
                ))
                .into());
            }
        }

        store.delete_record_by_wid(&entry.record.wid).await?;
        println!("Removed {}", entry.wid_short());
        Ok(CommandResult::Success(String::new()))
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "rm"
    }

    fn description(&self) -> &str {
        "Remove a detached runtime instance record"
    }
}
