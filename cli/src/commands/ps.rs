use crate::commands::runtime_state::{RuntimeStateStore, RuntimeStatus, resolve_hyper_dir};
use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use anyhow::Result;
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

    /// Show log file path column
    #[arg(long = "log")]
    pub log: bool,
}

#[async_trait]
impl Command for PsCommand {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let mut entries = store.list_records().await?;

        if !self.all {
            entries.retain(|entry| entry.status == RuntimeStatus::Running);
        }

        if entries.is_empty() {
            println!("No detached runtimes found.");
            return Ok(CommandResult::Success(String::new()));
        }

        let mut table = Table::new();
        let mut header = vec![
            Cell::new("WID").add_attribute(Attribute::Bold),
            Cell::new("ACTR_ID").add_attribute(Attribute::Bold),
            Cell::new("PID").add_attribute(Attribute::Bold),
            Cell::new("STATUS").add_attribute(Attribute::Bold),
            Cell::new("STARTED_AT").add_attribute(Attribute::Bold),
        ];
        if self.log {
            header.push(Cell::new("LOG").add_attribute(Attribute::Bold));
        }
        table.set_header(header);

        for entry in entries {
            let started_at = entry.started_at_display();
            let log_path = entry.record.log_path.display().to_string();
            let mut row = vec![
                Cell::new(entry.wid_short()),
                Cell::new(&entry.record.actr_id),
                Cell::new(entry.record.pid),
                Cell::new(entry.status.as_str()),
                Cell::new(started_at),
            ];
            if self.log {
                row.push(Cell::new(log_path));
            }
            table.add_row(row);
        }

        println!("{table}");
        Ok(CommandResult::Success(String::new()))
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "ps"
    }

    fn description(&self) -> &str {
        "List detached runtime instances"
    }
}
