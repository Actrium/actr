use crate::commands::process::{kill_process, terminate_process, wait_for_exit};
use crate::commands::runtime_state::{RuntimeStateStore, RuntimeStatus, resolve_hyper_dir};
use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use crate::error::ActrCliError;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use clap::Args;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args, Debug)]
pub struct StopCommand {
    /// WID (or unique prefix, min 8 chars) of the runtime to stop
    #[arg(value_name = "WID")]
    pub wid: String,

    /// Runtime configuration file
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Hyper data directory
    #[arg(long = "hyper-dir", value_name = "DIR")]
    pub hyper_dir: Option<PathBuf>,

    /// Graceful shutdown timeout in seconds
    #[arg(long = "timeout", default_value_t = 5)]
    pub timeout: u64,

    /// Send SIGKILL after graceful shutdown timeout
    #[arg(long = "force")]
    pub force: bool,
}

#[async_trait]
impl Command for StopCommand {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let entry = store.resolve_wid_prefix(&self.wid).await?;
        let wid = entry.record.wid.clone();
        let wid_short = entry.wid_short();
        let pid = entry.record.pid;

        // Mark the runtime as stopped in the state store, then print `message`.
        let finish = |msg: String| {
            let store = &store;
            let wid = wid.clone();
            async move {
                store.mark_stopped_by_wid(&wid, Utc::now()).await?;
                println!("{msg}");
                crate::error::Result::Ok(())
            }
        };

        if entry.status != RuntimeStatus::Running {
            finish(format!("Runtime already stopped: {wid_short}")).await?;
            return Ok(CommandResult::Success(String::new()));
        }

        if !terminate_process(pid)? {
            finish(format!("Runtime already stopped: {wid_short}")).await?;
            return Ok(CommandResult::Success(String::new()));
        }
        if wait_for_exit(pid, Duration::from_secs(self.timeout)).await {
            finish(format!("Stopped runtime: {wid_short}")).await?;
            return Ok(CommandResult::Success(String::new()));
        }

        if !self.force {
            return Err(ActrCliError::command_error(format!(
                "Timed out after {}s while stopping {}. Retry with --force.",
                self.timeout, wid_short
            ))
            .into());
        }

        if !kill_process(pid)? {
            finish(format!("Runtime already stopped: {wid_short}")).await?;
            return Ok(CommandResult::Success(String::new()));
        }
        if wait_for_exit(pid, Duration::from_secs(1)).await {
            finish(format!("Force stopped runtime: {wid_short}")).await?;
            return Ok(CommandResult::Success(String::new()));
        }

        Err(ActrCliError::command_error(format!("Process {pid} did not exit after SIGKILL")).into())
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "stop"
    }

    fn description(&self) -> &str {
        "Stop a detached runtime instance"
    }
}
