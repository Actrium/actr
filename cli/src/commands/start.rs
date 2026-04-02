//! Start command - re-launch a stopped detached runtime instance

use crate::commands::Command;
use crate::commands::run::RunCommand;
use crate::commands::runtime_state::{RuntimeStateStore, RuntimeStatus, resolve_hyper_dir};
use crate::error::{ActrCliError, Result};
use async_trait::async_trait;
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct StartCommand {
    /// WID (or unique prefix, min 8 chars) of the runtime to start
    #[arg(value_name = "WID")]
    pub wid: String,

    /// Override runtime configuration file
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Hyper data directory
    #[arg(long = "hyper-dir", value_name = "DIR")]
    pub hyper_dir: Option<PathBuf>,
}

#[async_trait]
impl Command for StartCommand {
    async fn execute(&self) -> Result<()> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let entry = store.resolve_wid_prefix(&self.wid).await?;

        if entry.status == RuntimeStatus::Running {
            return Err(ActrCliError::command_error(format!(
                "Runtime {} is already running (pid {}). Use `restart` to restart it.",
                entry.wid_short(),
                entry.record.pid
            )));
        }

        let config_path = self
            .config
            .clone()
            .unwrap_or_else(|| entry.record.config_path.clone());

        RunCommand {
            config: Some(config_path),
            detach: true,
            internal_detached_child: false,
            internal_wid: Some(entry.record.wid.clone()),
        }
        .execute()
        .await
    }
}
