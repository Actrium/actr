use super::*;

fn ok_bytes(s: &str) -> ActorResult<Bytes> {
    Ok(Bytes::from(s.as_bytes().to_vec()))
}

#[test]
fn fresh_request_is_marked_and_concurrent_duplicate_returns_waiter() {
    let mut d = DedupState::new();
    assert!(matches!(d.check_or_mark("req-1"), DedupOutcome::Fresh));
    assert_eq!(d.len(), 1);
    assert!(matches!(
        d.check_or_mark("req-1"),
        DedupOutcome::InFlight(_)
    ));
}

#[tokio::test]
async fn in_flight_duplicate_waiter_receives_original_result() {
    let mut d = DedupState::new();
    assert!(matches!(d.check_or_mark("req-1"), DedupOutcome::Fresh));

    let mut waiter = match d.check_or_mark("req-1") {
        DedupOutcome::InFlight(waiter) => waiter,
        other => panic!("expected InFlight waiter, got {other:?}"),
    };

    d.complete("req-1", ok_bytes("hello"));
    let _ = waiter.changed().await;

    let result = waiter
        .borrow()
        .clone()
        .expect("waiter should observe completed result");
    assert!(
        matches!(result, Ok(ref b) if b == "hello"),
        "expected waiter to receive original Ok(\"hello\")"
    );
}

#[test]
fn in_flight_entry_is_not_evicted_by_completed_cache_ttl() {
    let mut d = DedupState {
        ttl: Duration::from_nanos(1),
        ..DedupState::new()
    };
    assert!(matches!(d.check_or_mark("req-slow"), DedupOutcome::Fresh));

    // A different request triggers lazy eviction. The in-flight entry must
    // stay protected even when completed-cache TTL is tiny.
    d.check_or_mark("req-other");
    assert!(matches!(
        d.check_or_mark("req-slow"),
        DedupOutcome::InFlight(_)
    ));
}

#[test]
fn dedup_ttl_covers_reliable_rpc_retry_window() {
    assert!(
        DEDUP_TTL >= Duration::from_secs(20),
        "dedup TTL should cover late RpcReliable retries"
    );
}

#[test]
fn completed_duplicate_returns_cached_success_or_error() {
    let mut d = DedupState::new();
    d.check_or_mark("req-1");
    d.complete("req-1", ok_bytes("hello"));

    let outcome = d.check_or_mark("req-1");
    assert!(
        matches!(outcome, DedupOutcome::Duplicate(Ok(ref b)) if b == "hello"),
        "expected cached Ok(\"hello\")"
    );

    use actr_protocol::ActrError;
    let mut d = DedupState::new();
    d.check_or_mark("req-err");
    d.complete(
        "req-err",
        Err(ActrError::InvalidArgument("bad input".to_string())),
    );

    let outcome = d.check_or_mark("req-err");
    assert!(
        matches!(
            outcome,
            DedupOutcome::Duplicate(Err(ActrError::InvalidArgument(_)))
        ),
        "expected cached Err"
    );
}

#[test]
fn expired_entry_is_evicted_and_treated_as_fresh() {
    let mut d = DedupState {
        ttl: Duration::from_nanos(1), // expire immediately
        ..DedupState::new()
    };
    d.check_or_mark("req-old");
    d.complete("req-old", ok_bytes("v1"));

    // Force TTL expiry by advancing: we can't time-travel Instant in stable Rust,
    // so set TTL to 0 and trigger eviction on the next check.
    // Insert a fresh entry to trigger evict_expired, then re-check the old one.
    d.check_or_mark("req-new"); // triggers evict with ttl=1ns, old entry expires
    assert!(matches!(d.check_or_mark("req-old"), DedupOutcome::Fresh));
}

#[test]
fn different_request_ids_are_independent() {
    let mut d = DedupState::new();
    d.check_or_mark("req-a");
    d.complete("req-a", ok_bytes("a"));

    assert!(matches!(d.check_or_mark("req-b"), DedupOutcome::Fresh));
    assert!(matches!(
        d.check_or_mark("req-a"),
        DedupOutcome::Duplicate(_)
    ));
}
