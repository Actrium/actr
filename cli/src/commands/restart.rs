//! Restart command - stop a running instance and re-launch it

use crate::commands::run::RunCommand;
use crate::commands::runtime_state::{RuntimeStateStore, resolve_hyper_dir};
use crate::commands::stop::StopCommand;
use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use anyhow::Result;
use async_trait::async_trait;
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct RestartCommand {
    /// WID (or unique prefix, min 8 chars) of the runtime to restart
    #[arg(value_name = "WID")]
    pub wid: String,

    /// Override runtime configuration file
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
impl Command for RestartCommand {
    async fn execute(&self, ctx: &CommandContext) -> Result<CommandResult> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let entry = store.resolve_wid_prefix(&self.wid).await?;

        let full_wid = entry.record.wid.clone();
        let config_path = self
            .config
            .clone()
            .unwrap_or_else(|| entry.record.config_path.clone());

        println!("Stopping runtime: {}", entry.wid_short());
        StopCommand {
            wid: full_wid.clone(),
            config: self.config.clone(),
            hyper_dir: self.hyper_dir.clone(),
            timeout: self.timeout,
            force: self.force,
        }
        .execute(ctx)
        .await?;

        println!("Starting runtime with config: {}", config_path.display());
        RunCommand {
            config: Some(config_path),
            hyper_dir: self.hyper_dir.clone(),
            detach: true,
            internal_detached_child: false,
            internal_wid: Some(full_wid),
            web: false,
            port: None,
        }
        .execute(ctx)
        .await
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "restart"
    }

    fn description(&self) -> &str {
        "Restart a detached runtime instance"
    }
}
