//! `actr dlq` — Dead Letter Queue inspection and remediation.
//!
//! Subcommands:
//!   - `list`   — list DLQ records (default: newest 20)
//!   - `show`   — show full detail for one record
//!   - `stats`  — print DLQ statistics
//!   - `replay` — re-enqueue a record's raw bytes into a mailbox
//!   - `purge`  — permanently delete records (by ID, or by filter with `--all`)

use actr_runtime_mailbox::{
    DeadLetterQueue,
    dlq::{DlqQuery, DlqRecord},
    mailbox::{Mailbox, MessagePriority},
    sqlite::SqliteMailbox,
    sqlite_dlq::SqliteDeadLetterQueue,
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::DateTime;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::core::{Command, CommandContext, CommandResult, ComponentType};

const DEFAULT_DB_PATH: &str = "actr-data/dlq.db";
const DEFAULT_MAILBOX_PATH: &str = "actr-data/mailbox.db";
const DEFAULT_LIST_LIMIT: u32 = 20;

#[derive(Args, Debug)]
pub struct DlqArgs {
    #[command(subcommand)]
    pub command: DlqCommand,
}

#[derive(Subcommand, Debug)]
pub enum DlqCommand {
    /// List DLQ records (newest first).
    List(DlqListArgs),
    /// Show full detail for a single record.
    Show(DlqShowArgs),
    /// Print aggregate statistics.
    Stats(DlqStatsArgs),
    /// Re-enqueue a record's raw bytes into a live mailbox.
    Replay(DlqReplayArgs),
    /// Permanently remove records.
    Purge(DlqPurgeArgs),
}

#[derive(Args, Debug)]
pub struct DlqListArgs {
    /// Path to DLQ SQLite file
    #[arg(long, default_value = DEFAULT_DB_PATH, value_name = "FILE")]
    pub db: PathBuf,
    /// Max records to return
    #[arg(long, default_value_t = DEFAULT_LIST_LIMIT)]
    pub limit: u32,
    /// Filter by error_category
    #[arg(long, value_name = "CATEGORY")]
    pub category: Option<String>,
    /// Filter records created after timestamp (RFC 3339)
    #[arg(long, value_name = "RFC3339")]
    pub after: Option<String>,
}

#[derive(Args, Debug)]
pub struct DlqShowArgs {
    /// DLQ record UUID
    #[arg(value_name = "ID")]
    pub id: String,
    /// Path to DLQ SQLite file
    #[arg(long, default_value = DEFAULT_DB_PATH, value_name = "FILE")]
    pub db: PathBuf,
}

#[derive(Args, Debug)]
pub struct DlqStatsArgs {
    /// Path to DLQ SQLite file
    #[arg(long, default_value = DEFAULT_DB_PATH, value_name = "FILE")]
    pub db: PathBuf,
}

#[derive(Args, Debug)]
pub struct DlqReplayArgs {
    /// DLQ record UUID
    #[arg(value_name = "ID")]
    pub id: String,
    /// Path to DLQ SQLite file
    #[arg(long, default_value = DEFAULT_DB_PATH, value_name = "FILE")]
    pub db: PathBuf,
    /// Path to target mailbox SQLite file
    #[arg(long, default_value = DEFAULT_MAILBOX_PATH, value_name = "FILE")]
    pub mailbox: PathBuf,
    /// Keep the DLQ record after a successful replay (default: delete)
    #[arg(long)]
    pub keep: bool,
}

#[derive(Args, Debug)]
pub struct DlqPurgeArgs {
    /// Single record UUID to purge. If omitted, `--all` is required.
    #[arg(value_name = "ID")]
    pub id: Option<String>,
    /// Path to DLQ SQLite file
    #[arg(long, default_value = DEFAULT_DB_PATH, value_name = "FILE")]
    pub db: PathBuf,
    /// Purge every record. Combine with `--category` or `--before` to narrow scope.
    #[arg(long, conflicts_with = "id")]
    pub all: bool,
    /// When purging with `--all`, restrict to records in this error_category
    #[arg(long, value_name = "CATEGORY", requires = "all")]
    pub category: Option<String>,
    /// When purging with `--all`, restrict to records created before the timestamp (RFC 3339)
    #[arg(long, value_name = "RFC3339", requires = "all")]
    pub before: Option<String>,
}

#[async_trait]
impl Command for DlqArgs {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        match &self.command {
            DlqCommand::List(a) => cmd_list(a).await?,
            DlqCommand::Show(a) => cmd_show(a).await?,
            DlqCommand::Stats(a) => cmd_stats(a).await?,
            DlqCommand::Replay(a) => cmd_replay(a).await?,
            DlqCommand::Purge(a) => cmd_purge(a).await?,
        }
        Ok(CommandResult::Success(String::new()))
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "dlq"
    }

    fn description(&self) -> &str {
        "Dead Letter Queue inspection and remediation"
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

async fn open_dlq(db: &Path) -> Result<SqliteDeadLetterQueue> {
    SqliteDeadLetterQueue::new_standalone(db)
        .await
        .with_context(|| format!("Failed to open DLQ database at {}", db.display()))
}

fn parse_id(raw: &str) -> Result<Uuid> {
    Uuid::parse_str(raw).with_context(|| format!("Invalid UUID: '{raw}'"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn print_record_summary(r: &DlqRecord) {
    println!(
        "{id}  {ts}  {cat:<24}  {msg}",
        id = r.id,
        ts = r.created_at.format("%Y-%m-%d %H:%M:%SZ"),
        cat = r.error_category,
        msg = truncate(&r.error_message, 60),
    );
}

fn print_record_detail(r: &DlqRecord) {
    println!("ID:              {}", r.id);
    println!("Created at:      {}", r.created_at.to_rfc3339());
    println!("Error category:  {}", r.error_category);
    println!("Error message:   {}", r.error_message);
    println!("Trace ID:        {}", r.trace_id);
    if let Some(ref rid) = r.request_id {
        println!("Request ID:      {rid}");
    }
    if let Some(ref mid) = r.original_message_id {
        println!("Message ID:      {mid}");
    }
    println!("Raw bytes:       {} bytes", r.raw_bytes.len());
    if !r.raw_bytes.is_empty() {
        let preview: String = r
            .raw_bytes
            .iter()
            .take(32)
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let suffix = if r.raw_bytes.len() > 32 { " …" } else { "" };
        println!("                 {preview}{suffix}");
    }
    println!("Redrive attempts:{}", r.redrive_attempts);
    if let Some(ref ts) = r.last_redrive_at {
        println!("Last redrive:    {}", ts.to_rfc3339());
    }
    if let Some(ref ctx) = r.context {
        println!("Context:         {ctx}");
    }
}

fn parse_rfc3339(s: &str, flag: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.to_utc())
        .with_context(|| {
            format!("{flag} must be a valid RFC 3339 timestamp (e.g. 2026-01-01T00:00:00Z)")
        })
}

// ── subcommands ──────────────────────────────────────────────────────────────

async fn cmd_list(args: &DlqListArgs) -> Result<()> {
    let dlq = open_dlq(&args.db).await?;

    let after = args
        .after
        .as_deref()
        .map(|s| parse_rfc3339(s, "--after"))
        .transpose()?;

    let query = DlqQuery {
        error_category: args.category.clone(),
        limit: Some(args.limit),
        created_after: after,
        ..Default::default()
    };

    let records = dlq.query(query).await.context("DLQ query failed")?;

    if records.is_empty() {
        println!("DLQ is empty (no matching records).");
        return Ok(());
    }

    println!(
        "{:<36}  {:<20}  {:<24}  Error",
        "ID", "Created at", "Category"
    );
    println!("{}", "-".repeat(110));
    for r in &records {
        print_record_summary(r);
    }
    println!("\n{} record(s) shown (limit={})", records.len(), args.limit);
    Ok(())
}

async fn cmd_show(args: &DlqShowArgs) -> Result<()> {
    let id = parse_id(&args.id)?;
    let dlq = open_dlq(&args.db).await?;
    match dlq.get(id).await.context("DLQ get failed")? {
        Some(r) => {
            print_record_detail(&r);
            Ok(())
        }
        None => bail!("No DLQ record found with ID: {id}"),
    }
}

async fn cmd_stats(args: &DlqStatsArgs) -> Result<()> {
    let dlq = open_dlq(&args.db).await?;
    let stats = dlq.stats().await.context("DLQ stats failed")?;

    println!("DLQ Statistics");
    println!("  Total messages:           {}", stats.total_messages);
    println!(
        "  With redrive attempts:    {}",
        stats.messages_with_redrive_attempts
    );
    if let Some(ts) = stats.oldest_message_at {
        println!("  Oldest message:           {}", ts.to_rfc3339());
    }
    if !stats.messages_by_category.is_empty() {
        println!("  By category:");
        let mut cats: Vec<_> = stats.messages_by_category.iter().collect();
        cats.sort_by(|a, b| b.1.cmp(a.1));
        for (cat, count) in cats {
            println!("    {cat:<30} {count}");
        }
    }
    Ok(())
}

async fn cmd_replay(args: &DlqReplayArgs) -> Result<()> {
    let id = parse_id(&args.id)?;
    let dlq = open_dlq(&args.db).await?;
    let record = dlq
        .get(id)
        .await
        .context("DLQ get failed")?
        .ok_or_else(|| anyhow::anyhow!("No DLQ record found with ID: {id}"))?;

    if !args.mailbox.exists() {
        bail!(
            "Target mailbox file does not exist: {}\n\
             Specify a different path with --mailbox.",
            args.mailbox.display()
        );
    }

    let from = record.from.clone().ok_or_else(|| {
        anyhow::anyhow!("DLQ record {id} has no 'from' ActrId; cannot re-enqueue without a sender.")
    })?;

    let mailbox = SqliteMailbox::new(&args.mailbox)
        .await
        .with_context(|| format!("Failed to open mailbox: {}", args.mailbox.display()))?;

    let msg_id = mailbox
        .enqueue(from, record.raw_bytes.clone(), MessagePriority::Normal)
        .await
        .context("Failed to re-enqueue into mailbox")?;

    dlq.record_redrive_attempt(id)
        .await
        .context("Failed to record redrive attempt")?;

    if args.keep {
        println!(
            "Replayed DLQ record {id} into {} as message {msg_id} (kept in DLQ).",
            args.mailbox.display()
        );
    } else {
        dlq.delete(id)
            .await
            .context("Failed to delete DLQ record")?;
        println!(
            "Replayed DLQ record {id} into {} as message {msg_id} and removed from DLQ.",
            args.mailbox.display()
        );
    }
    Ok(())
}

async fn cmd_purge(args: &DlqPurgeArgs) -> Result<()> {
    let dlq = open_dlq(&args.db).await?;

    if let Some(id) = &args.id {
        let uuid = parse_id(id)?;
        if dlq.get(uuid).await.context("DLQ get failed")?.is_none() {
            bail!("No DLQ record found with ID: {uuid}");
        }
        dlq.delete(uuid).await.context("DLQ delete failed")?;
        println!("Purged DLQ record: {uuid}");
        return Ok(());
    }

    if !args.all {
        bail!("Specify a record ID, or pass --all (optionally with --category / --before).");
    }

    let before = args
        .before
        .as_deref()
        .map(|s| parse_rfc3339(s, "--before"))
        .transpose()?;

    // Query matching records, then delete each. We restrict to DlqQuery's filters
    // (category, created_after); `created_before` is applied client-side.
    let query = DlqQuery {
        error_category: args.category.clone(),
        limit: None,
        created_after: None,
        ..Default::default()
    };
    let records = dlq.query(query).await.context("DLQ query failed")?;

    let mut purged = 0usize;
    for r in records {
        if let Some(cutoff) = before
            && r.created_at >= cutoff
        {
            continue;
        }
        dlq.delete(r.id).await.context("DLQ delete failed")?;
        purged += 1;
    }

    println!("Purged {purged} DLQ record(s).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_runtime_mailbox::{dlq::DlqRecord, sqlite_dlq::SqliteDeadLetterQueue};
    use chrono::Utc;
    use tempfile::tempdir;

    async fn make_dlq() -> (SqliteDeadLetterQueue, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dlq.db");
        let dlq = SqliteDeadLetterQueue::new_standalone(&path).await.unwrap();
        (dlq, dir)
    }

    fn sample_record(category: &str, msg: &str) -> DlqRecord {
        DlqRecord {
            id: uuid::Uuid::new_v4(),
            original_message_id: None,
            from: Some(b"sender-actr-id".to_vec()),
            to: None,
            raw_bytes: b"bad bytes".to_vec(),
            error_message: msg.to_string(),
            error_category: category.to_string(),
            trace_id: uuid::Uuid::new_v4().to_string(),
            request_id: None,
            created_at: Utc::now(),
            redrive_attempts: 0,
            last_redrive_at: None,
            context: None,
        }
    }

    #[tokio::test]
    async fn list_returns_records() {
        let (dlq, dir) = make_dlq().await;
        dlq.enqueue(sample_record("decode", "bad proto"))
            .await
            .unwrap();
        let db = dir.path().join("dlq.db");
        cmd_list(&DlqListArgs {
            db,
            limit: 10,
            category: None,
            after: None,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn show_missing_record_returns_error() {
        let (_dlq, dir) = make_dlq().await;
        let db = dir.path().join("dlq.db");
        assert!(
            cmd_show(&DlqShowArgs {
                id: Uuid::new_v4().to_string(),
                db,
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn purge_by_id_deletes() {
        let (dlq, dir) = make_dlq().await;
        let id = dlq.enqueue(sample_record("decode", "x")).await.unwrap();
        let db = dir.path().join("dlq.db");
        cmd_purge(&DlqPurgeArgs {
            id: Some(id.to_string()),
            db: db.clone(),
            all: false,
            category: None,
            before: None,
        })
        .await
        .unwrap();
        let dlq2 = SqliteDeadLetterQueue::new_standalone(&db).await.unwrap();
        assert!(dlq2.get(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn purge_all_with_category_filter() {
        let (dlq, dir) = make_dlq().await;
        dlq.enqueue(sample_record("decode", "a")).await.unwrap();
        dlq.enqueue(sample_record("envelope", "b")).await.unwrap();
        let db = dir.path().join("dlq.db");
        cmd_purge(&DlqPurgeArgs {
            id: None,
            db: db.clone(),
            all: true,
            category: Some("decode".into()),
            before: None,
        })
        .await
        .unwrap();
        let dlq2 = SqliteDeadLetterQueue::new_standalone(&db).await.unwrap();
        let remaining = dlq2.query(DlqQuery::default()).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].error_category, "envelope");
    }

    #[tokio::test]
    async fn replay_moves_record_to_mailbox() {
        let (dlq, dir) = make_dlq().await;
        let id = dlq.enqueue(sample_record("decode", "x")).await.unwrap();
        let db = dir.path().join("dlq.db");
        let mailbox = dir.path().join("mailbox.db");
        // touch mailbox file first via SqliteMailbox::new
        let _ = SqliteMailbox::new(&mailbox).await.unwrap();

        cmd_replay(&DlqReplayArgs {
            id: id.to_string(),
            db: db.clone(),
            mailbox: mailbox.clone(),
            keep: false,
        })
        .await
        .unwrap();

        let dlq2 = SqliteDeadLetterQueue::new_standalone(&db).await.unwrap();
        assert!(dlq2.get(id).await.unwrap().is_none());
    }
}
