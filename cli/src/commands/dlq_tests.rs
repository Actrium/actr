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
