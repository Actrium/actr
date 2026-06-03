//! Request deduplication with TTL-based response caching.
//!
//! ## Design
//!
//! When a caller retries a request (e.g. after a transient send failure), the receiver
//! may process the same `request_id` twice, leading to double side-effects.
//!
//! `DedupState` prevents this by caching the response for each `request_id` for a
//! fixed TTL (default 30 s).  A second call with the same `request_id` within the TTL
//! returns the cached response without re-invoking the handler.
//! If the original request is still in-flight, duplicate callers wait for the
//! original result instead of receiving a synthetic duplicate error.
//!
//! ## Eviction
//!
//! Entries older than the TTL are evicted lazily on every `check_or_mark` call.
//! No background task is required.

use actr_framework::Bytes;
use actr_protocol::ActorResult;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// Default TTL: must be comfortably longer than the longest RpcReliable retry window.
/// RpcReliable: 5 attempts, 1s + 2s + 4s + 5s gaps = 12s of retry backoff,
/// plus connection/recovery timing. Keep enough margin so a late retry still
/// sees the completed cache instead of re-running the handler.
pub(crate) const DEDUP_TTL: Duration = Duration::from_secs(30);

pub(crate) type DedupWaiter = watch::Receiver<Option<ActorResult<Bytes>>>;

/// Cached outcome of a completed request.
#[derive(Clone, Debug)]
enum CachedResult {
    /// Request processed and response stored.
    Done(ActorResult<Bytes>),
    /// Request is currently in-flight (guards against concurrent duplicates).
    InFlight {
        completion_tx: watch::Sender<Option<ActorResult<Bytes>>>,
    },
}

/// Entry stored per request_id.
#[derive(Debug)]
struct Entry {
    received_at: Instant,
    result: CachedResult,
}

/// Framework-side request deduplication state.
///
/// Not `Clone` intentionally; share via `Arc<Mutex<DedupState>>`.
#[derive(Debug, Default)]
pub(crate) struct DedupState {
    entries: HashMap<String, Entry>,
    ttl: Duration,
}

/// Outcome of `check_or_mark`.
#[derive(Debug)]
pub(crate) enum DedupOutcome {
    /// First time we see this request_id; proceed with handling.
    Fresh,
    /// Request is already being processed; wait for the original result.
    InFlight(DedupWaiter),
    /// Request was already processed; the cached response is returned.
    Duplicate(ActorResult<Bytes>),
}

impl DedupState {
    /// Create a dedup state with the default 30 s TTL.
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
            ttl: DEDUP_TTL,
        }
    }

    /// Check whether `request_id` is a duplicate.
    ///
    /// - If fresh: inserts an `InFlight` marker and returns `DedupOutcome::Fresh`.
    /// - If already in-flight: returns a waiter for the original result.
    /// - If already completed and still in TTL: returns the cached result.
    /// - Expired completed entries are evicted on every call.
    pub(crate) fn check_or_mark(&mut self, request_id: &str) -> DedupOutcome {
        let now = Instant::now();
        self.evict_expired(now);

        match self.entries.get(request_id) {
            None => {
                let (completion_tx, _completion_rx) = watch::channel(None);
                self.entries.insert(
                    request_id.to_string(),
                    Entry {
                        received_at: now,
                        result: CachedResult::InFlight { completion_tx },
                    },
                );
                DedupOutcome::Fresh
            }
            Some(entry) => match &entry.result {
                CachedResult::InFlight { completion_tx } => {
                    DedupOutcome::InFlight(completion_tx.subscribe())
                }
                CachedResult::Done(r) => DedupOutcome::Duplicate(r.clone()),
            },
        }
    }

    /// Record the completed response for `request_id`.
    ///
    /// Call this after the handler finishes (success or error).
    pub(crate) fn complete(&mut self, request_id: &str, result: ActorResult<Bytes>) {
        if let Some(entry) = self.entries.get_mut(request_id) {
            if let CachedResult::InFlight { completion_tx } = &entry.result {
                let _ = completion_tx.send(Some(result.clone()));
            }
            entry.result = CachedResult::Done(result);
        }
    }

    /// Evict completed entries that have exceeded the TTL.
    ///
    /// In-flight entries are retained until completion. Evicting them on the
    /// completed-cache TTL would allow a slow handler to be re-entered by a
    /// retry with the same request_id.
    fn evict_expired(&mut self, now: Instant) {
        let ttl = self.ttl;
        self.entries.retain(|_, e| match e.result {
            CachedResult::InFlight { .. } => true,
            CachedResult::Done(_) => now.duration_since(e.received_at) < ttl,
        });
    }

    /// Number of entries currently tracked (for monitoring / tests).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
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
}
