use crate::commands::Command;
use crate::commands::runtime_state::{RuntimeStateStore, RuntimeStatus, resolve_hyper_dir};
use crate::error::Result;
use async_trait::async_trait;
use clap::Args;
use comfy_table::{Attribute, Cell, Table};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct PsCommand {
    /// Runtime configuration file
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Hyper data directory
    #[arg(long = "hyper-dir", value_name = "DIR")]
    pub hyper_dir: Option<PathBuf>,

    /// Show running, exited, and stale instances
    #[arg(long = "all")]
    pub all: bool,
}

#[async_trait]
impl Command for PsCommand {
    async fn execute(&self) -> Result<()> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let mut entries = store.list_records().await?;

        if !self.all {
            entries.retain(|entry| entry.status == RuntimeStatus::Running);
        }

        if entries.is_empty() {
            println!("No detached runtimes found.");
            return Ok(());
        }

        let mut table = Table::new();
        table.set_header(vec![
            Cell::new("ACTR_ID").add_attribute(Attribute::Bold),
            Cell::new("PID").add_attribute(Attribute::Bold),
            Cell::new("STATUS").add_attribute(Attribute::Bold),
            Cell::new("STARTED_AT").add_attribute(Attribute::Bold),
            Cell::new("LOG").add_attribute(Attribute::Bold),
        ]);

        for entry in entries {
            let started_at = entry.started_at_display();
            let log_path = entry.record.log_path.display().to_string();
            table.add_row(vec![
                Cell::new(entry.record.actr_id),
                Cell::new(entry.record.pid),
                Cell::new(entry.status.as_str()),
                Cell::new(started_at),
                Cell::new(log_path),
            ]);
        }

        println!("{table}");
        Ok(())
    }
}
