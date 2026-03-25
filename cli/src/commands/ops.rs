//! `actr ops` — Operations commands
//!
//! Groups operational commands like dlq.

use anyhow::Result;
use clap::{Args, Subcommand};

use super::dlq;

#[derive(Args, Debug)]
pub struct OpsArgs {
    #[command(subcommand)]
    pub command: OpsCommand,
}

#[derive(Subcommand, Debug)]
pub enum OpsCommand {
    /// Dead Letter Queue inspection
    Dlq(OpsDlqArgs),
}

/// Arguments for `actr ops dlq`
#[derive(clap::Args, Debug)]
pub struct OpsDlqArgs {
    /// Subcommand: list | show | stats | delete  [default: list]
    #[arg(value_name = "SUBCOMMAND")]
    pub subcommand: Option<String>,

    /// Record ID (required for show/delete)
    #[arg(value_name = "ID")]
    pub id: Option<String>,

    /// Path to DLQ SQLite file
    #[arg(long, default_value = "actr-data/dlq.db")]
    pub db: std::path::PathBuf,

    /// Max records to return for 'list'
    #[arg(long, default_value_t = 20)]
    pub limit: u32,

    /// Filter by error_category
    #[arg(long)]
    pub category: Option<String>,

    /// Filter records created after timestamp (RFC 3339)
    #[arg(long)]
    pub after: Option<String>,
}

pub async fn execute(args: OpsArgs) -> Result<()> {
    match args.command {
        OpsCommand::Dlq(dlq_args) => {
            let inner_args = dlq::DlqArgs {
                subcommand: dlq_args.subcommand.unwrap_or_else(|| "list".to_string()),
                id: dlq_args.id,
                db: dlq_args.db,
                limit: dlq_args.limit,
                category: dlq_args.category,
                after: dlq_args.after,
            };
            dlq::execute(inner_args).await
        }
    }
}
