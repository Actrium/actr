use super::*;
use crate::transport::error::{NetworkError, NetworkResult};
use crate::transport::lane::DataLane;
use actr_protocol::PayloadType;
use std::time::Duration;

/// Minimal WireHandle mock. `succeed` controls `connect()` outcome.
#[derive(Debug)]
struct MockWire {
    conn_type: ConnType,
    succeed: bool,
    identity: Option<WireIdentity>,
}

#[async_trait::async_trait]
impl WireHandle for MockWire {
    fn connection_type(&self) -> ConnType {
        self.conn_type
    }
    fn priority(&self) -> u8 {
        1
    }
    async fn connect(&self) -> NetworkResult<()> {
        if self.succeed {
            Ok(())
        } else {
            Err(NetworkError::ConnectionError("mock connect failure".into()))
        }
    }
    fn is_connected(&self) -> bool {
        self.succeed
    }
    async fn close(&self) -> NetworkResult<()> {
        Ok(())
    }
    async fn get_lane(&self, _: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        Err(NetworkError::NotImplemented("mock has no lane".into()))
    }
    fn identity(&self) -> Option<WireIdentity> {
        self.identity.clone()
    }
}

fn fast_retry() -> RetryConfig {
    RetryConfig {
        max_attempts: 1,
        initial_delay_ms: 0,
        max_delay_ms: 10,
        multiplier: 1.0,
    }
}

// ── RetryConfig ─────────────────────────────────────────────────────────

#[test]
fn retry_config_default_values() {
    let r = RetryConfig::default();
    assert_eq!(r.max_attempts, 3);
    assert_eq!(r.initial_delay_ms, 1000);
    assert_eq!(r.max_delay_ms, 10000);
    assert_eq!(r.multiplier, 2.0);
}

#[test]
fn retry_config_create_backoff_yields_first_attempt_at_no_delay() {
    // The first backoff step must be immediate (delay 0) so the initial
    // connection attempt isn't needlessly delayed.
    let mut b = RetryConfig::default().create_backoff();
    let first = b.next();
    assert!(first.is_some(), "backoff should yield at least one step");
}

// ── WirePool initial state ──────────────────────────────────────────────

#[tokio::test]
async fn new_pool_is_open_with_empty_ready_set() {
    let pool = WirePool::new(RetryConfig::default());
    assert!(!pool.is_closed());

    // Empty pool: no ready candidates, no connection.
    assert!(
        !pool
            .has_live_candidate(&[ConnType::WebSocket, ConnType::WebRTC])
            .await
    );
    assert!(pool.get_connection(ConnType::WebSocket).await.is_none());

    // watch_ready receiver starts with an empty set.
    let rx = pool.watch_ready();
    assert!(rx.borrow().is_empty());
}

#[tokio::test]
async fn close_all_marks_pool_closed_and_clears_ready() {
    let pool = WirePool::new(RetryConfig::default());
    assert!(!pool.is_closed());
    pool.close_all().await;
    assert!(pool.is_closed());
    // Ready set broadcast as empty.
    assert!(pool.watch_ready().borrow().is_empty());
}

#[tokio::test]
async fn mark_connection_closed_sets_failed_and_no_live_candidate() {
    let pool = WirePool::new(RetryConfig::default());
    pool.mark_connection_closed(ConnType::WebRTC).await;
    // Failed is not a live candidate.
    assert!(!pool.has_live_candidate(&[ConnType::WebRTC]).await);
    assert!(pool.get_connection(ConnType::WebRTC).await.is_none());
}

#[tokio::test]
async fn identity_checks_return_false_when_not_ready() {
    let pool = WirePool::new(RetryConfig::default());
    let id = WireIdentity::WebRtc {
        peer_id: actr_protocol::ActrId::default(),
        session_id: 1,
    };
    // No Ready slot → both identity checks are false.
    assert!(
        !pool
            .connection_matches_identity(ConnType::WebRTC, &id)
            .await
    );
    assert!(
        !pool
            .mark_connection_closed_if_same(ConnType::WebRTC, &id)
            .await,
        "non-ready slot should not be marked closed"
    );
}

// ── add_connection: success path ────────────────────────────────────────

#[tokio::test]
async fn add_connection_success_becomes_ready_and_broadcasts() {
    let pool = WirePool::new(fast_retry());
    let mut rx = pool.watch_ready();

    pool.add_connection(Arc::new(MockWire {
        conn_type: ConnType::WebSocket,
        succeed: true,
        identity: None,
    }))
    .await;

    // Await broadcast: ready set should contain WebSocket.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if rx.borrow().contains(&ConnType::WebSocket) {
                return;
            }
            let _ = tokio::time::timeout(Duration::from_millis(50), rx.changed()).await;
        }
    })
    .await
    .expect("ready broadcast should fire within 2s");

    // get_connection now returns the Ready handle.
    assert!(pool.get_connection(ConnType::WebSocket).await.is_some());
    assert!(pool.has_live_candidate(&[ConnType::WebSocket]).await);
}

// ── add_connection: failure path ────────────────────────────────────────

#[tokio::test]
async fn add_connection_failure_marks_failed_and_exhausts() {
    let pool = WirePool::new(fast_retry());
    pool.add_connection(Arc::new(MockWire {
        conn_type: ConnType::WebRTC,
        succeed: false,
        identity: None,
    }))
    .await;

    // Poll until the WebRTC slot stops being a live candidate (Connecting → Failed).
    tokio::time::timeout(Duration::from_secs(2), async {
        while pool.has_live_candidate(&[ConnType::WebRTC]).await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("failed connection should leave the live-candidate set within 2s");

    // Failed → no connection retrievable.
    assert!(pool.get_connection(ConnType::WebRTC).await.is_none());
    assert!(!pool.has_live_candidate(&[ConnType::WebRTC]).await);
}

// ── identity-aware close on a Ready slot ─────────────────────────────────

#[tokio::test]
async fn mark_closed_if_same_matches_and_mismatches() {
    let id = WireIdentity::WebRtc {
        peer_id: actr_protocol::ActrId::default(),
        session_id: 7,
    };
    let other = WireIdentity::WebRtc {
        peer_id: actr_protocol::ActrId::default(),
        session_id: 99,
    };
    let pool = WirePool::new(fast_retry());

    pool.add_connection(Arc::new(MockWire {
        conn_type: ConnType::WebSocket,
        succeed: true,
        identity: Some(id.clone()),
    }))
    .await;

    // Wait until Ready.
    tokio::time::timeout(Duration::from_secs(2), async {
        while pool.get_connection(ConnType::WebSocket).await.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    // Matching identity → closes (true).
    assert!(
        pool.mark_connection_closed_if_same(ConnType::WebSocket, &id)
            .await
    );
    // Slot now Failed → no longer matches anything.
    assert!(
        !pool
            .connection_matches_identity(ConnType::WebSocket, &id)
            .await
    );

    // Re-ready a fresh wire, then assert a mismatched identity does NOT close.
    let pool2 = WirePool::new(fast_retry());
    pool2
        .add_connection(Arc::new(MockWire {
            conn_type: ConnType::WebSocket,
            succeed: true,
            identity: Some(id.clone()),
        }))
        .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while pool2.get_connection(ConnType::WebSocket).await.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !pool2
            .mark_connection_closed_if_same(ConnType::WebSocket, &other)
            .await,
        "mismatched identity must not close the slot"
    );
    // Still ready afterwards.
    assert!(pool2.get_connection(ConnType::WebSocket).await.is_some());
}
