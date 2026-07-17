use super::*;
use crate::transport::ConnType;
use crate::transport::lane::DataLane;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::{Duration, timeout};

struct TestFactory;

#[async_trait]
impl WireBuilder for TestFactory {
    async fn create_connections(&self, _dest: &Dest) -> NetworkResult<Vec<Arc<dyn WireHandle>>> {
        // Test factory: returns empty list (real usage requires actual connections)
        Ok(vec![])
    }
}

fn create_test_factory() -> Arc<dyn WireBuilder> {
    Arc::new(TestFactory)
}

#[tokio::test]
async fn test_transport_manager_creation() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id.clone(), factory);

    assert_eq!(mgr.dest_count().await, 0);
    assert_eq!(mgr.local_id(), &local_id);
}

#[tokio::test]
async fn test_list_dests() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id, factory);

    let dests = mgr.list_dests().await;
    assert_eq!(dests.len(), 0);
}

#[tokio::test]
async fn test_has_dest() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id, factory);

    let dest = Dest::host();
    assert!(!mgr.has_dest(&dest).await);
}

#[tokio::test]
async fn close_transport_if_current_replaced_instance_does_not_mark_closing() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id, factory);
    let dest = Dest::host();

    let old_transport = Arc::new(
        DestTransport::new(dest.clone(), vec![])
            .await
            .expect("old transport should be created"),
    );
    let current_transport = Arc::new(
        DestTransport::new(dest.clone(), vec![])
            .await
            .expect("current transport should be created"),
    );
    let old_ref = DestTransportRef::new(&old_transport, None);

    mgr.transports
        .write()
        .await
        .insert(dest.clone(), DestState::Connected(current_transport));

    let closed = mgr
        .close_transport_if_current(&dest, &old_ref)
        .await
        .expect("stale instance close should not fail");

    assert!(!closed);
    assert_eq!(mgr.dest_count().await, 1);
    assert!(
        !mgr.is_closing(&dest).await,
        "stale no-op close must not mark the replacement transport closing"
    );
}

// ── close_webrtc_transport_if_session: WebRTC-only teardown, WS kept alive ──

/// Minimal wire that succeeds on `connect`, counts `close` calls, and optionally
/// carries a `WireIdentity` (so `matches_webrtc_session` can match).
#[derive(Debug)]
struct CountingWire {
    conn_type: ConnType,
    identity: Option<WireIdentity>,
    close_count: Arc<AtomicUsize>,
}

impl CountingWire {
    fn new(conn_type: ConnType, identity: Option<WireIdentity>) -> Self {
        Self {
            conn_type,
            identity,
            close_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn close_count(&self) -> usize {
        self.close_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl WireHandle for CountingWire {
    fn connection_type(&self) -> ConnType {
        self.conn_type
    }

    fn priority(&self) -> u8 {
        match self.conn_type {
            ConnType::WebSocket => 0,
            ConnType::WebRTC => 1,
        }
    }

    async fn connect(&self) -> NetworkResult<()> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        true
    }

    async fn close(&self) -> NetworkResult<()> {
        self.close_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn get_lane(&self, _payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        Ok(Arc::new(NoopLane))
    }

    fn identity(&self) -> Option<WireIdentity> {
        self.identity.clone()
    }
}

#[derive(Debug)]
struct NoopLane;

#[async_trait]
impl DataLane for NoopLane {
    async fn send(&self, _data: bytes::Bytes) -> NetworkResult<()> {
        Ok(())
    }

    fn lane_type(&self) -> &'static str {
        "noop"
    }
}

struct SupersedableFactory {
    calls: AtomicUsize,
    first_started: Notify,
    first_release: Notify,
    first_wire: Arc<CountingWire>,
    replacement_wire: Arc<CountingWire>,
}

#[async_trait]
impl WireBuilder for SupersedableFactory {
    async fn create_connections(&self, _dest: &Dest) -> NetworkResult<Vec<Arc<dyn WireHandle>>> {
        let call = self.calls.fetch_add(1, Ordering::AcqRel);
        if call == 0 {
            self.first_started.notify_one();
            self.first_release.notified().await;
            Ok(vec![self.first_wire.clone()])
        } else {
            Ok(vec![self.replacement_wire.clone()])
        }
    }
}

#[tokio::test]
async fn superseded_creator_cannot_overwrite_replacement_flight() {
    let first_wire = Arc::new(CountingWire::new(ConnType::WebSocket, None));
    let replacement_wire = Arc::new(CountingWire::new(ConnType::WebSocket, None));
    let factory = Arc::new(SupersedableFactory {
        calls: AtomicUsize::new(0),
        first_started: Notify::new(),
        first_release: Notify::new(),
        first_wire: first_wire.clone(),
        replacement_wire,
    });
    let manager = Arc::new(PeerTransport::new(ActrId::default(), factory.clone()));
    let dest = Dest::host();

    let first_manager = manager.clone();
    let first_dest = dest.clone();
    let first =
        tokio::spawn(async move { first_manager.get_or_create_transport(&first_dest).await });
    timeout(Duration::from_secs(1), factory.first_started.notified())
        .await
        .expect("first flight should start");

    manager
        .close_transport(&dest)
        .await
        .expect("closing first flight should succeed");

    let replacement = timeout(
        Duration::from_secs(1),
        manager.get_or_create_transport(&dest),
    )
    .await
    .expect("replacement flight should not wait for stale creator")
    .expect("replacement flight should succeed");

    factory.first_release.notify_one();
    let stale_result = timeout(Duration::from_secs(1), first)
        .await
        .expect("stale creator should finish")
        .expect("stale creator task should not panic");
    assert!(
        stale_result.is_err(),
        "stale creator must not commit over replacement"
    );

    let current = manager
        .get_or_create_transport(&dest)
        .await
        .expect("current transport should remain available");
    assert!(Arc::ptr_eq(&current, &replacement));
    assert_eq!(factory.calls.load(Ordering::Acquire), 2);
    assert_eq!(
        first_wire.close_count(),
        1,
        "transport produced by stale creator must be closed"
    );
}

#[tokio::test]
async fn cancelled_creator_releases_flight_without_waiter_timeout() {
    let factory = Arc::new(SupersedableFactory {
        calls: AtomicUsize::new(0),
        first_started: Notify::new(),
        first_release: Notify::new(),
        first_wire: Arc::new(CountingWire::new(ConnType::WebSocket, None)),
        replacement_wire: Arc::new(CountingWire::new(ConnType::WebSocket, None)),
    });
    let manager = Arc::new(PeerTransport::new(ActrId::default(), factory.clone()));
    let dest = Dest::host();

    let first_manager = manager.clone();
    let first_dest = dest.clone();
    let first =
        tokio::spawn(async move { first_manager.get_or_create_transport(&first_dest).await });
    timeout(Duration::from_secs(1), factory.first_started.notified())
        .await
        .expect("first flight should start");

    first.abort();
    let _ = first.await;

    timeout(
        Duration::from_secs(1),
        manager.get_or_create_transport(&dest),
    )
    .await
    .expect("replacement must wake from cancellation rather than the ten-second waiter timeout")
    .expect("replacement flight should succeed");
    assert_eq!(
        factory.calls.load(Ordering::Acquire),
        2,
        "replacement should own a fresh generation"
    );
}

#[tokio::test]
async fn flight_waiter_is_event_driven_and_cannot_miss_completion() {
    let first_wire = Arc::new(CountingWire::new(ConnType::WebSocket, None));
    let factory = Arc::new(SupersedableFactory {
        calls: AtomicUsize::new(0),
        first_started: Notify::new(),
        first_release: Notify::new(),
        first_wire,
        replacement_wire: Arc::new(CountingWire::new(ConnType::WebSocket, None)),
    });
    let manager = Arc::new(PeerTransport::new(ActrId::default(), factory.clone()));
    let dest = Dest::host();

    let creator_manager = manager.clone();
    let creator_dest = dest.clone();
    let creator =
        tokio::spawn(async move { creator_manager.get_or_create_transport(&creator_dest).await });
    timeout(Duration::from_secs(1), factory.first_started.notified())
        .await
        .expect("creator should start");

    let waiter_manager = manager.clone();
    let waiter_dest = dest.clone();
    let waiter =
        tokio::spawn(async move { waiter_manager.get_or_create_transport(&waiter_dest).await });
    factory.first_release.notify_one();

    let created = timeout(Duration::from_secs(1), creator)
        .await
        .expect("creator should finish")
        .expect("creator should not panic")
        .expect("creator should succeed");
    let observed = timeout(Duration::from_secs(1), waiter)
        .await
        .expect("waiter should wake from flight completion")
        .expect("waiter should not panic")
        .expect("waiter should succeed");

    assert!(Arc::ptr_eq(&created, &observed));
    assert_eq!(
        factory.calls.load(Ordering::Acquire),
        1,
        "waiter must share the existing flight"
    );
}

/// Build a `PeerTransport` whose `dest` maps to a ready `DestTransport` carrying
/// both a WebRTC wire (with the given session identity) and a WebSocket wire.
///
/// Returns the manager and the `dest` key; the caller already holds `Arc`s to
/// the two `CountingWire`s so it can assert on their `close_count` directly.
async fn peer_transport_with_webrtc_and_ws(
    peer_id: ActrId,
    session_id: u64,
    webrtc: Arc<CountingWire>,
    websocket: Arc<CountingWire>,
) -> (PeerTransport, Dest) {
    let mgr = PeerTransport::new(peer_id.clone(), create_test_factory());
    let dest = Dest::host();

    let transport = DestTransport::new(dest.clone(), vec![webrtc, websocket])
        .await
        .expect("transport should be created");

    // `DestTransport::new` spawns background `connect` tasks; wait for the
    // WebRTC wire to become Ready so `matches_webrtc_session` can observe it.
    let ready = timeout(Duration::from_secs(2), async {
        loop {
            if transport.matches_webrtc_session(&peer_id, session_id).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(ready.is_ok(), "WebRTC wire should become Ready in time");

    mgr.transports
        .write()
        .await
        .insert(dest.clone(), DestState::Connected(Arc::new(transport)));

    (mgr, dest)
}

#[tokio::test]
async fn close_webrtc_transport_if_session_closes_only_webrtc_keeps_websocket() {
    let peer_id = ActrId::default();
    let session_id = 42u64;

    let webrtc = Arc::new(CountingWire::new(
        ConnType::WebRTC,
        Some(WireIdentity::WebRtc {
            peer_id: peer_id.clone(),
            session_id,
        }),
    ));
    let websocket = Arc::new(CountingWire::new(ConnType::WebSocket, None));

    let (mgr, dest) = peer_transport_with_webrtc_and_ws(
        peer_id.clone(),
        session_id,
        webrtc.clone(),
        websocket.clone(),
    )
    .await;

    let closed = mgr
        .close_webrtc_transport_if_session(&dest, &peer_id, session_id)
        .await
        .expect("session-matched close should not error");

    assert!(closed, "matching session should close the WebRTC wire");
    assert_eq!(
        webrtc.close_count(),
        1,
        "WebRTC wire should be closed exactly once"
    );
    assert_eq!(
        websocket.close_count(),
        0,
        "WebSocket fallback wire must NOT be closed by a WebRTC-only teardown"
    );
}

#[tokio::test]
async fn close_webrtc_transport_if_session_skips_on_stale_session() {
    let peer_id = ActrId::default();
    let active_session = 42u64;
    let stale_session = 7u64;

    let webrtc = Arc::new(CountingWire::new(
        ConnType::WebRTC,
        Some(WireIdentity::WebRtc {
            peer_id: peer_id.clone(),
            session_id: active_session,
        }),
    ));
    let websocket = Arc::new(CountingWire::new(ConnType::WebSocket, None));

    let (mgr, dest) = peer_transport_with_webrtc_and_ws(
        peer_id.clone(),
        active_session,
        webrtc.clone(),
        websocket.clone(),
    )
    .await;

    // A stale close event referencing a different session must be a no-op.
    let closed = mgr
        .close_webrtc_transport_if_session(&dest, &peer_id, stale_session)
        .await
        .expect("stale close should not error");

    assert!(!closed, "stale session close should be skipped");
    assert_eq!(
        webrtc.close_count(),
        0,
        "WebRTC wire must NOT be closed for a stale session"
    );
    assert_eq!(
        websocket.close_count(),
        0,
        "WebSocket wire must NOT be closed for a stale session"
    );
}
