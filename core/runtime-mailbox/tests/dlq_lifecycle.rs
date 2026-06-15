//! DLQ integration test — full enqueue → query → redrive → purge lifecycle
//!
//! This test exercises the SQLite-backed Dead Letter Queue end-to-end without
//! requiring the full runtime stack. It validates:
//!
//! 1. **enqueue** — a corrupt-envelope record is stored durably
//! 2. **query by category** — `error_category` filter returns correct subset
//! 3. **query by trace_id** — span correlation lookup works
//! 4. **stats** — aggregated counts and per-category breakdown are accurate
//! 5. **redrive tracking** — `record_redrive_attempt` increments counter and
//!    sets `last_redrive_at` without removing the record
//! 6. **delete (purge)** — confirmed entry disappears after `delete(id)`
//! 7. **multi-record ordering** — `query` returns newest-first (DESC created_at)
//!
//! # DLQ dispatch wiring status (as of 2026-04-27)
//!
//! The SQLite DLQ **is** wired into the dispatch path:
//! `core/hyper/src/lifecycle/node.rs:2188` enqueues a `DlqRecord` whenever
//! `prost::decode` fails for an inbound mailbox message
//! (`error_category = "protobuf_decode"`).
//!
//! `requires_dlq()` in `core/hyper/src/transport/error.rs` is defined on
//! `NetworkError` for `Corrupt` variants, but that code-path (transport-layer
//! corrupted envelope) does **not** currently call `dlq.enqueue` — only the
//! mailbox dispatch loop does. Tracking issue: "transport-level DLQ routing".

use actr_runtime_mailbox::{DeadLetterQueue, DlqQuery, DlqRecord, DlqStats, SqliteDeadLetterQueue};
use chrono::Utc;
use tempfile::tempdir;
use uuid::Uuid;

// ── helpers ───────────────────────────────────────────────────────────────────

async fn open_dlq() -> SqliteDeadLetterQueue {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_dlq.db");
    // `tempdir` is dropped here but the path lives long enough because we only
    // keep `SqliteDeadLetterQueue` (which holds an open Connection).
    // Leak the tempdir to keep the file alive for the test duration.
    let path = {
        let p = path.to_owned();
        std::mem::forget(dir);
        p
    };
    SqliteDeadLetterQueue::new_standalone(&path).await.unwrap()
}

fn make_record(category: &str, trace_id: &str) -> DlqRecord {
    DlqRecord {
        id: Uuid::new_v4(),
        original_message_id: Some(format!("msg-{}", Uuid::new_v4())),
        from: Some(vec![0xAA, 0xBB]),
        to: Some(vec![0xCC, 0xDD]),
        raw_bytes: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00],
        error_message: format!("Simulated decode failure [{category}]"),
        error_category: category.to_string(),
        trace_id: trace_id.to_string(),
        request_id: Some("req-test-001".to_string()),
        created_at: Utc::now(),
        redrive_attempts: 0,
        last_redrive_at: None,
        context: Some(r#"{"transport":"mailbox","test":true}"#.to_string()),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Basic round-trip: enqueue a poison record and retrieve it by ID.
#[tokio::test]
async fn dlq_enqueue_and_get_roundtrip() {
    let dlq = open_dlq().await;
    let record = make_record("protobuf_decode", "trace-abc-001");
    let id = record.id;

    let returned_id = dlq.enqueue(record.clone()).await.unwrap();
    assert_eq!(returned_id, id, "enqueue must return the record's own UUID");

    let fetched = dlq
        .get(id)
        .await
        .unwrap()
        .expect("record must exist after enqueue");
    assert_eq!(fetched.id, id);
    assert_eq!(fetched.error_category, "protobuf_decode");
    assert_eq!(fetched.trace_id, "trace-abc-001");
    assert_eq!(fetched.redrive_attempts, 0);
    assert!(fetched.last_redrive_at.is_none());
}

/// `query` with `error_category` filter returns only matching records.
#[tokio::test]
async fn dlq_query_by_category_filters_correctly() {
    let dlq = open_dlq().await;

    let r1 = make_record("protobuf_decode", "trace-1");
    let r2 = make_record("corrupted_envelope", "trace-2");
    let r3 = make_record("protobuf_decode", "trace-3");

    dlq.enqueue(r1).await.unwrap();
    dlq.enqueue(r2).await.unwrap();
    dlq.enqueue(r3).await.unwrap();

    let results = dlq
        .query(DlqQuery {
            error_category: Some("protobuf_decode".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(results.len(), 2);
    for r in &results {
        assert_eq!(r.error_category, "protobuf_decode");
    }
}

/// `query` with `trace_id` filter returns only the matching span.
#[tokio::test]
async fn dlq_query_by_trace_id() {
    let dlq = open_dlq().await;

    let target_trace = "trace-unique-xyz-9999";
    dlq.enqueue(make_record("protobuf_decode", target_trace))
        .await
        .unwrap();
    dlq.enqueue(make_record("protobuf_decode", "trace-other-1"))
        .await
        .unwrap();
    dlq.enqueue(make_record("protobuf_decode", "trace-other-2"))
        .await
        .unwrap();

    let results = dlq
        .query(DlqQuery {
            trace_id: Some(target_trace.to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].trace_id, target_trace);
}

/// `stats()` returns correct aggregates: total count, per-category breakdown,
/// and redrive-attempt count.
#[tokio::test]
async fn dlq_stats_aggregation() {
    let dlq = open_dlq().await;

    let r1 = make_record("protobuf_decode", "t1");
    let r1_id = r1.id;
    let r2 = make_record("corrupted_envelope", "t2");
    let r2_id = r2.id;
    let r3 = make_record("protobuf_decode", "t3");

    dlq.enqueue(r1).await.unwrap();
    dlq.enqueue(r2).await.unwrap();
    dlq.enqueue(r3).await.unwrap();

    // Record one redrive attempt for each of the first two
    dlq.record_redrive_attempt(r1_id).await.unwrap();
    dlq.record_redrive_attempt(r2_id).await.unwrap();

    let stats: DlqStats = dlq.stats().await.unwrap();

    assert_eq!(stats.total_messages, 3);
    assert_eq!(stats.messages_with_redrive_attempts, 2);
    assert_eq!(
        stats
            .messages_by_category
            .get("protobuf_decode")
            .copied()
            .unwrap_or(0),
        2
    );
    assert_eq!(
        stats
            .messages_by_category
            .get("corrupted_envelope")
            .copied()
            .unwrap_or(0),
        1
    );
    assert!(stats.oldest_message_at.is_some());
}

/// `record_redrive_attempt` increments the counter and sets `last_redrive_at`
/// without removing the record from the DLQ.
#[tokio::test]
async fn dlq_redrive_tracking_increments_counter() {
    let dlq = open_dlq().await;
    let record = make_record("protobuf_decode", "trace-redrive");
    let id = record.id;
    dlq.enqueue(record).await.unwrap();

    // Initial state
    let initial = dlq.get(id).await.unwrap().unwrap();
    assert_eq!(initial.redrive_attempts, 0);
    assert!(initial.last_redrive_at.is_none());

    // First redrive attempt
    dlq.record_redrive_attempt(id).await.unwrap();
    let after_1 = dlq.get(id).await.unwrap().unwrap();
    assert_eq!(after_1.redrive_attempts, 1);
    assert!(
        after_1.last_redrive_at.is_some(),
        "last_redrive_at must be set after first attempt"
    );

    // Second redrive attempt
    dlq.record_redrive_attempt(id).await.unwrap();
    let after_2 = dlq.get(id).await.unwrap().unwrap();
    assert_eq!(after_2.redrive_attempts, 2);

    // Record still present after redrive
    let still_there = dlq.get(id).await.unwrap();
    assert!(
        still_there.is_some(),
        "record must remain in DLQ after redrive attempt"
    );
}

/// `delete` (purge) removes the record permanently.
#[tokio::test]
async fn dlq_delete_purges_record() {
    let dlq = open_dlq().await;
    let record = make_record("protobuf_decode", "trace-purge");
    let id = record.id;

    dlq.enqueue(record).await.unwrap();
    assert!(
        dlq.get(id).await.unwrap().is_some(),
        "record must exist before delete"
    );

    dlq.delete(id).await.unwrap();
    assert!(
        dlq.get(id).await.unwrap().is_none(),
        "record must be gone after delete"
    );

    // Stats should reflect the deletion
    let stats = dlq.stats().await.unwrap();
    assert_eq!(stats.total_messages, 0);
}

/// Multiple records are returned in descending `created_at` order (newest first).
#[tokio::test]
async fn dlq_query_order_newest_first() {
    let dlq = open_dlq().await;

    // Enqueue three records with explicit timestamps to guarantee ordering
    use chrono::Duration as ChronoDuration;
    let now = Utc::now();

    let mut old_record = make_record("protobuf_decode", "old");
    old_record.created_at = now - ChronoDuration::seconds(60);

    let mut mid_record = make_record("protobuf_decode", "mid");
    mid_record.created_at = now - ChronoDuration::seconds(30);

    let mut new_record = make_record("protobuf_decode", "new");
    new_record.created_at = now;

    // Enqueue in non-chronological order
    dlq.enqueue(mid_record.clone()).await.unwrap();
    dlq.enqueue(old_record.clone()).await.unwrap();
    dlq.enqueue(new_record.clone()).await.unwrap();

    let results = dlq.query(DlqQuery::default()).await.unwrap();
    assert_eq!(results.len(), 3);

    // Verify descending order
    assert_eq!(results[0].trace_id, "new", "newest record must come first");
    assert_eq!(results[1].trace_id, "mid");
    assert_eq!(results[2].trace_id, "old", "oldest record must come last");
}

/// `query` with `limit` returns at most N records.
#[tokio::test]
async fn dlq_query_respects_limit() {
    let dlq = open_dlq().await;

    for i in 0..5 {
        dlq.enqueue(make_record("protobuf_decode", &format!("trace-{i}")))
            .await
            .unwrap();
    }

    let results = dlq
        .query(DlqQuery {
            limit: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(results.len(), 2, "query must respect the limit parameter");
}
