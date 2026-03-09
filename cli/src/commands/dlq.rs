//! `actr dlq` — Dead Letter Queue inspection tool
//!
//! Provides read/delete access to the SQLite DLQ for operator use.
//!
//! ## Subcommands
//!
//! ```text
//! actr dlq list   [--db=PATH] [--limit=N] [--category=CAT] [--after=RFC3339]
//! actr dlq show   <ID> [--db=PATH]
//! actr dlq stats  [--db=PATH]
//! actr dlq delete <ID> [--db=PATH]
//! ```
//!
//! `--db` defaults to `./actr-data/dlq.db` (the runtime default path).

use actr_runtime_mailbox::{
    DeadLetterQueue,
    dlq::{DlqQuery, DlqRecord},
    sqlite_dlq::SqliteDeadLetterQueue,
};
use anyhow::{Context, Result, bail};
use chrono::DateTime;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const DEFAULT_DB_PATH: &str = "actr-data/dlq.db";
const DEFAULT_LIST_LIMIT: u32 = 20;

/// Arguments parsed from CLI flags for `actr dlq`.
pub struct DlqArgs {
    pub subcommand: String,
    /// Positional argument (ID for show/delete)
    pub id: Option<String>,
    pub db: PathBuf,
    pub limit: u32,
    pub category: Option<String>,
    pub after: Option<String>,
}

impl DlqArgs {
    pub fn parse(
        subcommand: Option<&str>,
        positional: &[String],
        flags: &std::collections::HashMap<String, String>,
    ) -> Result<Self> {
        let subcommand = subcommand.unwrap_or("list").to_string();
        let id = positional.first().cloned();
        let db = flags
            .get("db")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DB_PATH));
        let limit = flags
            .get("limit")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_LIST_LIMIT);
        let category = flags.get("category").cloned();
        let after = flags.get("after").cloned();

        Ok(Self {
            subcommand,
            id,
            db,
            limit,
            category,
            after,
        })
    }
}

/// Execute the `actr dlq` command.
pub async fn execute(args: DlqArgs) -> Result<()> {
    match args.subcommand.as_str() {
        "list" => cmd_list(&args).await,
        "show" => cmd_show(&args).await,
        "stats" => cmd_stats(&args).await,
        "delete" => cmd_delete(&args).await,
        other => bail!(
            "Unknown dlq subcommand '{}'. Use: list | show | stats | delete",
            other
        ),
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

async fn open_dlq(db: &Path) -> Result<SqliteDeadLetterQueue> {
    SqliteDeadLetterQueue::new_standalone(db)
        .await
        .with_context(|| format!("Failed to open DLQ database at {}", db.display()))
}

fn require_id(args: &DlqArgs) -> Result<Uuid> {
    let raw = args
        .id
        .as_deref()
        .context("Missing required <ID> argument")?;
    Uuid::parse_str(raw).with_context(|| format!("Invalid UUID: '{raw}'"))
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
    println!("Raw bytes (hex): {} bytes", r.raw_bytes.len());
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

// ── subcommands ───────────────────────────────────────────────────────────────

async fn cmd_list(args: &DlqArgs) -> Result<()> {
    let dlq = open_dlq(&args.db).await?;

    let after = args
        .after
        .as_deref()
        .map(|s| DateTime::parse_from_rfc3339(s).map(|dt| dt.to_utc()))
        .transpose()
        .context("--after must be a valid RFC 3339 timestamp (e.g. 2026-01-01T00:00:00Z)")?;

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

async fn cmd_show(args: &DlqArgs) -> Result<()> {
    let id = require_id(args)?;
    let dlq = open_dlq(&args.db).await?;

    match dlq.get(id).await.context("DLQ get failed")? {
        Some(r) => {
            print_record_detail(&r);
            Ok(())
        }
        None => bail!("No DLQ record found with ID: {id}"),
    }
}

async fn cmd_stats(args: &DlqArgs) -> Result<()> {
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

async fn cmd_delete(args: &DlqArgs) -> Result<()> {
    let id = require_id(args)?;
    let dlq = open_dlq(&args.db).await?;

    // Verify the record exists first so we give a clear error if not found.
    if dlq.get(id).await.context("DLQ get failed")?.is_none() {
        bail!("No DLQ record found with ID: {id}");
    }

    dlq.delete(id).await.context("DLQ delete failed")?;
    println!("Deleted DLQ record: {id}");
    Ok(())
}

// ── help text ─────────────────────────────────────────────────────────────────

pub fn print_dlq_help() {
    println!(
        r#"actr dlq — Dead Letter Queue inspection

Usage:
    actr dlq list   [OPTIONS]      List DLQ entries (default: newest 20)
    actr dlq show   <ID> [OPTIONS] Show full detail for one entry
    actr dlq stats  [OPTIONS]      Print DLQ statistics
    actr dlq delete <ID> [OPTIONS] Delete a resolved entry

Options:
    --db=PATH           Path to DLQ SQLite file  [default: actr-data/dlq.db]
    --limit=N           Max records to return for 'list'  [default: 20]
    --category=CAT      Filter by error_category
    --after=RFC3339     Filter records created after timestamp

Examples:
    actr dlq list
    actr dlq list --limit=50 --category=protobuf_decode
    actr dlq show 550e8400-e29b-41d4-a716-446655440000
    actr dlq stats
    actr dlq delete 550e8400-e29b-41d4-a716-446655440000
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_runtime_mailbox::{dlq::DlqRecord, sqlite_dlq::SqliteDeadLetterQueue};
    use chrono::Utc;
    use std::collections::HashMap;
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
            from: None,
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

    fn flags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // ── DlqArgs::parse ──────────────────────────────────────────────────────

    #[test]
    fn parse_defaults_to_list() {
        let args = DlqArgs::parse(None, &[], &HashMap::new()).unwrap();
        assert_eq!(args.subcommand, "list");
        assert_eq!(args.limit, DEFAULT_LIST_LIMIT);
        assert!(args.id.is_none());
    }

    #[test]
    fn parse_show_with_id() {
        let pos = vec!["550e8400-e29b-41d4-a716-446655440000".to_string()];
        let args = DlqArgs::parse(Some("show"), &pos, &HashMap::new()).unwrap();
        assert_eq!(args.subcommand, "show");
        assert_eq!(
            args.id.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn parse_list_with_flags() {
        let f = flags(&[("limit", "5"), ("category", "decode"), ("db", "/tmp/x.db")]);
        let args = DlqArgs::parse(Some("list"), &[], &f).unwrap();
        assert_eq!(args.limit, 5);
        assert_eq!(args.category.as_deref(), Some("decode"));
        assert_eq!(args.db, PathBuf::from("/tmp/x.db"));
    }

    #[test]
    fn require_id_returns_error_when_missing() {
        let args = DlqArgs::parse(Some("show"), &[], &HashMap::new()).unwrap();
        assert!(require_id(&args).is_err());
    }

    #[test]
    fn require_id_returns_error_for_bad_uuid() {
        let pos = vec!["not-a-uuid".to_string()];
        let args = DlqArgs::parse(Some("show"), &pos, &HashMap::new()).unwrap();
        assert!(require_id(&args).is_err());
    }

    // ── cmd_list ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_empty_db_prints_empty_message() {
        let (_, dir) = make_dlq().await;
        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "list".into(),
            id: None,
            db,
            limit: 20,
            category: None,
            after: None,
        };
        // Should not error even when DB is empty
        cmd_list(&args).await.unwrap();
    }

    #[tokio::test]
    async fn list_returns_records() {
        let (dlq, dir) = make_dlq().await;
        dlq.enqueue(sample_record("decode", "bad proto"))
            .await
            .unwrap();
        dlq.enqueue(sample_record("decode", "truncated"))
            .await
            .unwrap();

        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "list".into(),
            id: None,
            db,
            limit: 10,
            category: None,
            after: None,
        };
        cmd_list(&args).await.unwrap();
    }

    #[tokio::test]
    async fn list_filters_by_category() {
        let (dlq, dir) = make_dlq().await;
        dlq.enqueue(sample_record("decode", "bad proto"))
            .await
            .unwrap();
        dlq.enqueue(sample_record("envelope", "bad header"))
            .await
            .unwrap();

        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "list".into(),
            id: None,
            db,
            limit: 10,
            category: Some("envelope".into()),
            after: None,
        };
        cmd_list(&args).await.unwrap(); // just check no error; output goes to stdout
    }

    // ── cmd_show ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn show_existing_record() {
        let (dlq, dir) = make_dlq().await;
        let rec = sample_record("decode", "corrupted");
        let id = dlq.enqueue(rec.clone()).await.unwrap();

        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "show".into(),
            id: Some(id.to_string()),
            db,
            limit: 20,
            category: None,
            after: None,
        };
        cmd_show(&args).await.unwrap();
    }

    #[tokio::test]
    async fn show_missing_record_returns_error() {
        let (_dlq, dir) = make_dlq().await;
        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "show".into(),
            id: Some(Uuid::new_v4().to_string()),
            db,
            limit: 20,
            category: None,
            after: None,
        };
        assert!(cmd_show(&args).await.is_err());
    }

    // ── cmd_stats ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn stats_on_empty_db() {
        let (_dlq, dir) = make_dlq().await;
        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "stats".into(),
            id: None,
            db,
            limit: 20,
            category: None,
            after: None,
        };
        cmd_stats(&args).await.unwrap();
    }

    #[tokio::test]
    async fn stats_with_records() {
        let (dlq, dir) = make_dlq().await;
        dlq.enqueue(sample_record("decode", "a")).await.unwrap();
        dlq.enqueue(sample_record("decode", "b")).await.unwrap();
        dlq.enqueue(sample_record("envelope", "c")).await.unwrap();

        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "stats".into(),
            id: None,
            db,
            limit: 20,
            category: None,
            after: None,
        };
        cmd_stats(&args).await.unwrap();
    }

    // ── cmd_delete ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_existing_record() {
        let (dlq, dir) = make_dlq().await;
        let id = dlq
            .enqueue(sample_record("decode", "poison"))
            .await
            .unwrap();

        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "delete".into(),
            id: Some(id.to_string()),
            db: db.clone(),
            limit: 20,
            category: None,
            after: None,
        };
        cmd_delete(&args).await.unwrap();

        // Verify it's gone
        let dlq2 = SqliteDeadLetterQueue::new_standalone(&db).await.unwrap();
        assert!(dlq2.get(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_record_returns_error() {
        let (_dlq, dir) = make_dlq().await;
        let db = dir.path().join("dlq.db");
        let args = DlqArgs {
            subcommand: "delete".into(),
            id: Some(Uuid::new_v4().to_string()),
            db,
            limit: 20,
            category: None,
            after: None,
        };
        assert!(cmd_delete(&args).await.is_err());
    }

    // ── unknown subcommand ────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_subcommand_returns_error() {
        let args = DlqArgs {
            subcommand: "oops".into(),
            id: None,
            db: PathBuf::from("x.db"),
            limit: 20,
            category: None,
            after: None,
        };
        assert!(execute(args).await.is_err());
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_has_ellipsis() {
        let s = truncate("hello world", 5);
        assert!(s.ends_with('…'));
        assert!(s.len() <= 6 + '…'.len_utf8()); // 5 chars + ellipsis
    }
}
