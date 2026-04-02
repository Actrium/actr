use crate::commands::Command;
use crate::commands::process::{kill_process, terminate_process, wait_for_exit};
use crate::commands::runtime_state::{RuntimeStateStore, RuntimeStatus, resolve_hyper_dir};
use crate::error::{ActrCliError, Result};
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
    async fn execute(&self) -> Result<()> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let entry = store.resolve_wid_prefix(&self.wid).await?;

        if entry.status != RuntimeStatus::Running {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Runtime already stopped: {}", entry.wid_short());
            return Ok(());
        }

        if !terminate_process(entry.record.pid)? {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Runtime already stopped: {}", entry.wid_short());
            return Ok(());
        }
        if wait_for_exit(entry.record.pid, Duration::from_secs(self.timeout)).await {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Stopped runtime: {}", entry.wid_short());
            return Ok(());
        }

        if !self.force {
            return Err(ActrCliError::command_error(format!(
                "Timed out after {}s while stopping {}. Retry with --force.",
                self.timeout,
                entry.wid_short()
            )));
        }

        if !kill_process(entry.record.pid)? {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Runtime already stopped: {}", entry.wid_short());
            return Ok(());
        }
        if wait_for_exit(entry.record.pid, Duration::from_secs(1)).await {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Force stopped runtime: {}", entry.wid_short());
            return Ok(());
        }

        Err(ActrCliError::command_error(format!(
            "Process {} did not exit after SIGKILL",
            entry.record.pid
        )))
    }
}
