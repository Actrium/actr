//! Integration tests for PeerGate disconnection/reconnection
//!
//! Uses TestHarness for multi-peer topology with VNet network simulation.
//!
//! Tests focus on:
//! - Two-peer disconnect → network event → ICE restart → reconnect
//! - Offerer vs Answerer recovery latency comparison
//!
//! ## Recovery latency tests (Test 2 & 3)
//!
//! Both tests use a **short outage (8s)** so the connection stays in the
//! peers map and `do_ice_restart_inner` is still running (in its backoff loop).
//!
//! The key difference (Plan A implemented):
//! - **Offerer test**: offerer calls `retry_failed()` → `restart_ice()` → already inflight → wakes backoff
//! - **Answerer test**: answerer calls `retry_failed()` → `restart_ice()` → `!is_offerer`
//!   → sends IceRestartRequest → Offerer receives → wakes backoff → immediate retry

use actr_hyper::lifecycle::{
    DefaultNetworkEventProcessor, NetworkEvent, NetworkEventProcessor, NetworkRecoveryAction,
    process_network_event_batch, select_network_recovery_action,
};
use actr_hyper::test_support::TestHarness;
use actr_hyper::transport::{ConnectionEvent, ConnectionState, Dest};
use actr_protocol::{ActrId, PayloadType};
use std::time::{Duration, Instant};

/// Initialize tracing for test output
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_file(true)
        .with_line_number(true)
        .with_test_writer()
        .try_init()
        .ok();
}

async fn wait_for_data_channel_opened(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>,
    peer_id: &ActrId,
    payload_type: PayloadType,
    timeout: Duration,
) -> u64 {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for {:?} DataChannelOpened for peer {:?}",
            payload_type,
            peer_id
        );

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(ConnectionEvent::DataChannelOpened {
                peer_id: event_peer,
                session_id,
                payload_type: event_payload_type,
            })) if &event_peer == peer_id && event_payload_type == payload_type => {
                return session_id;
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Connection event receiver lagged by {} events", n);
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("connection event channel closed while waiting for DataChannelOpened");
            }
            Err(_) => {
                panic!(
                    "timed out waiting for {:?} DataChannelOpened for peer {:?}",
                    payload_type, peer_id
                );
            }
        }
    }
}

async fn wait_for_data_channel_close_chain(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>,
    peer_id: &ActrId,
    session_id: u64,
    timeout: Duration,
) -> PayloadType {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut closed_payload_type = None;
    let mut saw_peer_connection_closed = false;
    let mut saw_connection_closed = false;

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Connection event receiver lagged by {} events", n);
                continue;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("connection event channel closed while waiting for close chain");
            }
            Err(_) => break,
        };

        match event {
            ConnectionEvent::DataChannelClosed {
                peer_id: event_peer,
                session_id: event_session_id,
                payload_type,
            } if &event_peer == peer_id && event_session_id == session_id => {
                tracing::info!(
                    "Observed DataChannelClosed for peer {:?}, session_id={}, payload_type={:?}",
                    peer_id,
                    session_id,
                    payload_type
                );
                closed_payload_type.get_or_insert(payload_type);
            }
            ConnectionEvent::ConnectionClosed {
                peer_id: event_peer,
                session_id: event_session_id,
            } if &event_peer == peer_id && event_session_id == session_id => {
                tracing::info!(
                    "Observed ConnectionClosed for peer {:?}, session_id={}",
                    peer_id,
                    session_id
                );
                saw_connection_closed = true;
            }
            ConnectionEvent::StateChanged {
                peer_id: event_peer,
                session_id: event_session_id,
                state: ConnectionState::Closed,
            } if &event_peer == peer_id && event_session_id == session_id => {
                tracing::info!(
                    "Observed PeerConnection Closed for peer {:?}, session_id={}",
                    peer_id,
                    session_id
                );
                saw_peer_connection_closed = true;
            }
            _ => {}
        }

        if let Some(payload_type) = closed_payload_type
            && saw_peer_connection_closed
            && saw_connection_closed
        {
            return payload_type;
        }
    }

    panic!(
        "timed out waiting for DataChannelClosed -> PeerConnection Closed -> ConnectionClosed chain for peer {:?}, session_id={}, saw_data_channel_closed={}, saw_peer_connection_closed={}, saw_connection_closed={}",
        peer_id,
        session_id,
        closed_payload_type.is_some(),
        saw_peer_connection_closed,
        saw_connection_closed
    );
}

async fn wait_for_connection_closed(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>,
    peer_id: &ActrId,
    session_id: u64,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for ConnectionClosed for peer {:?}, session_id={}",
            peer_id,
            session_id
        );

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(ConnectionEvent::ConnectionClosed {
                peer_id: event_peer,
                session_id: event_session_id,
            })) if &event_peer == peer_id && event_session_id == session_id => return,
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Connection event receiver lagged by {} events", n);
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("connection event channel closed while waiting for ConnectionClosed");
            }
            Err(_) => {
                panic!(
                    "timed out waiting for ConnectionClosed for peer {:?}, session_id={}",
                    peer_id, session_id
                );
            }
        }
    }
}

async fn wait_for_peer_state(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>,
    peer_id: &ActrId,
    wanted_states: &[ConnectionState],
    timeout: Duration,
) -> (u64, ConnectionState) {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for peer {} to enter one of {:?}",
            peer_id,
            wanted_states
        );

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(ConnectionEvent::StateChanged {
                peer_id: event_peer,
                session_id,
                state,
            })) if &event_peer == peer_id && wanted_states.contains(&state) => {
                return (session_id, state);
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Connection event receiver lagged by {} events", n);
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("connection event channel closed while waiting for peer state");
            }
            Err(_) => {
                panic!(
                    "timed out waiting for peer {} to enter one of {:?}",
                    peer_id, wanted_states
                );
            }
        }
    }
}

async fn expect_connection_recovering(
    handle: tokio::task::JoinHandle<actr_protocol::ActorResult<actr_framework::Bytes>>,
    label: &str,
) {
    match tokio::time::timeout(Duration::from_secs(3), handle).await {
        Ok(Ok(Err(err))) => {
            let msg = err.to_string();
            assert!(
                msg.contains("Connection recovering"),
                "{label} failed, but not with Connection recovering: {msg}"
            );
        }
        Ok(Ok(Ok(response))) => {
            panic!(
                "{label} unexpectedly succeeded with {} response bytes",
                response.len()
            );
        }
        Ok(Err(err)) => panic!("{label} task panicked: {err}"),
        Err(_) => panic!("{label} did not finish within the outer timeout"),
    }
}

async fn expect_request_eventually_ok(
    harness: &TestHarness,
    from_serial: u64,
    to_serial: u64,
    request_prefix: &str,
    total_timeout: Duration,
    attempt_timeout_ms: u32,
) -> actr_framework::Bytes {
    let deadline = tokio::time::Instant::now() + total_timeout;
    let mut attempt = 0;

    loop {
        attempt += 1;
        let request_id = format!("{request_prefix}_{attempt}");
        let handle =
            harness
                .peer(from_serial)
                .spawn_request(to_serial, &request_id, attempt_timeout_ms);

        let last_error = match tokio::time::timeout(
            Duration::from_millis(attempt_timeout_ms as u64) + Duration::from_secs(1),
            handle,
        )
        .await
        {
            Ok(Ok(Ok(response))) => return response,
            Ok(Ok(Err(err))) => {
                let msg = err.to_string();
                assert!(
                    msg.contains("Connection recovering")
                        || msg.contains("Request timeout")
                        || msg.contains("Connection"),
                    "unexpected retry error while waiting for recovery: {msg}"
                );
                msg
            }
            Ok(Err(err)) => panic!("{request_prefix} retry task panicked: {err}"),
            Err(_) => format!("{request_prefix} attempt {attempt} timed out"),
        };

        if tokio::time::Instant::now() >= deadline {
            panic!(
                "{request_prefix} did not succeed within {:?}; last error: {}",
                total_timeout, last_error
            );
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_for_signaling_reconnect(
    harness: &TestHarness,
    min_connections: u32,
    min_disconnections: u32,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let connections = harness.server.get_connection_count();
        let disconnections = harness.server.get_disconnection_count();
        if connections >= min_connections && disconnections >= min_disconnections {
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for signaling reconnect counters: connections >= {}, disconnections >= {}; current connections={}, disconnections={}",
                min_connections, min_disconnections, connections, disconnections
            );
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ==================== DataChannel close cleanup ====================

#[tokio::test]
async fn test_answerer_network_change_requests_offerer_ice_restart() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("Step 1: Establishing connection with 100 as offerer and 200 as answerer");
    harness.connect(100, 200).await;
    harness.reset_counters();

    tracing::info!("Step 2: Processing network change on answerer peer 200");
    harness
        .peer(200)
        .network_processor()
        .process_network_type_changed(true, false)
        .await
        .expect("answerer network change should process successfully");

    let request_count = harness
        .wait_for_ice_restart_request_count(1, Duration::from_secs(5))
        .await;
    tracing::info!(
        "ICE restart request count after answerer network change: {}",
        request_count
    );

    let offer_count = harness
        .wait_for_ice_restart_count(1, Duration::from_secs(10))
        .await;
    tracing::info!(
        "ICE restart offer count after answerer request: {}",
        offer_count
    );

    let response = expect_request_eventually_ok(
        &harness,
        100,
        200,
        "answerer_requested_restart_verify",
        Duration::from_secs(10),
        2_000,
    )
    .await;
    tracing::info!(
        "Connection remained usable after answerer-requested ICE restart: {} bytes",
        response.len()
    );
}

#[tokio::test]
async fn test_answerer_ice_restart_answer_does_not_unblock_before_connected() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("Step 1: Establishing connection with 100 as offerer and 200 as answerer");
    harness.connect(100, 200).await;
    harness.reset_counters();

    let offerer_id = harness.peer(100).id.clone();
    let answerer = harness.peer(200);
    let mut answerer_events = answerer.subscribe_events();

    tracing::info!("Step 2: Keep signaling alive but block UDP before answerer recovery");
    harness
        .vnet
        .as_ref()
        .expect("test requires VNet")
        .block_network();

    tracing::info!("Step 3: Trigger answerer network recovery and wait until it answers restart");
    answerer
        .network_processor()
        .process_network_type_changed(true, false)
        .await
        .expect("answerer network change should process successfully");

    harness
        .wait_for_ice_restart_request_count(1, Duration::from_secs(5))
        .await;
    harness
        .wait_for_ice_restart_count(1, Duration::from_secs(10))
        .await;

    let (_session_id, state) = wait_for_peer_state(
        &mut answerer_events,
        &offerer_id,
        &[ConnectionState::Connecting],
        Duration::from_secs(5),
    )
    .await;
    tracing::info!(
        "Answerer observed restart negotiation state {:?} while UDP is still blocked",
        state
    );

    tokio::time::sleep(Duration::from_millis(100)).await;

    tracing::info!("Step 4: Sends must still fail fast until ICE reaches Connected");
    let early = answerer.spawn_request(100, "answerer-answer-before-connected", 500);
    expect_connection_recovering(early, "answerer send before ICE Connected").await;
}

#[tokio::test]
async fn test_stale_answerer_recovery_closes_old_session_on_network_restore() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("Step 1: Establishing connection with 100 as offerer and 200 as answerer");
    harness.connect(100, 200).await;
    harness.reset_counters();

    let offerer_id = harness.peer(100).id.clone();
    let answerer = harness.peer(200);
    let mut answerer_events = answerer.subscribe_events();

    tracing::info!("Step 2: Mark answerer peer 200 as recovering for a long outage");
    answerer
        .coordinator
        .begin_network_recovery("test long answerer recovery")
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let status = answerer
        .coordinator
        .peer_recovery_status(&offerer_id)
        .await
        .expect("answerer should guard the existing offerer session");
    let session_id = status.session_id;
    assert!(
        answerer
            .coordinator
            .force_peer_recovery_started_at_for_test(
                &offerer_id,
                Instant::now() - Duration::from_secs(61),
            )
            .await,
        "test should be able to age the answerer recovery guard"
    );

    tracing::info!("Step 3: Network restore should close stale answerer session, not request ICE");
    answerer
        .coordinator
        .restart_network_recovery_connections()
        .await;

    wait_for_connection_closed(
        &mut answerer_events,
        &offerer_id,
        session_id,
        Duration::from_secs(3),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        answerer
            .coordinator
            .peer_recovery_status(&offerer_id)
            .await
            .is_none(),
        "stale answerer recovery should clear the coordinator guard"
    );
    assert_eq!(
        harness.server.get_ice_restart_request_count(),
        0,
        "stale answerer recovery should close locally instead of asking offerer to restart ICE"
    );
}

#[tokio::test]
async fn test_duplicate_network_recovery_same_session_is_coalesced() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    harness.connect(100, 200).await;

    let peer_100 = harness.peer(100);
    let target_id = harness.peer(200).id.clone();
    let mut event_rx = peer_100.subscribe_events();

    let first_targets = peer_100
        .coordinator
        .begin_network_recovery("first network event")
        .await;
    assert_eq!(
        first_targets,
        vec![target_id.clone()],
        "first network event should mark the active peer for recovery"
    );

    let first_session_id = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match event_rx.recv().await {
                Ok(ConnectionEvent::IceRestartStarted {
                    peer_id,
                    session_id,
                }) if peer_id == target_id => return session_id,
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("connection event channel closed")
                }
            }
        }
    })
    .await
    .expect("first recovery event should emit IceRestartStarted");

    let aged_started_at = Instant::now() - Duration::from_secs(3);
    assert!(
        peer_100
            .coordinator
            .force_peer_recovery_started_at_for_test(&target_id, aged_started_at)
            .await,
        "test should be able to age the recovery guard"
    );

    let second_targets = peer_100
        .coordinator
        .begin_network_recovery("second network event")
        .await;
    assert!(
        second_targets.is_empty(),
        "duplicate network event for the same session should be coalesced"
    );

    let status = peer_100
        .coordinator
        .peer_recovery_status(&target_id)
        .await
        .expect("recovery guard should remain active");
    assert_eq!(status.session_id, first_session_id);
    assert_eq!(status.reason, "first network event");
    assert!(
        status.elapsed() >= Duration::from_secs(3),
        "duplicate recovery should not refresh the guard timer"
    );

    let duplicate = tokio::time::timeout(Duration::from_millis(150), async {
        loop {
            match event_rx.recv().await {
                Ok(ConnectionEvent::IceRestartStarted { .. }) => return,
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    })
    .await;
    assert!(
        duplicate.is_err(),
        "duplicate network event should not emit another IceRestartStarted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_signaling_restore_wakes_existing_restart_without_duplicate_offer() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("Step 1: Establish connection with peer 100 as offerer");
    harness.connect(100, 200).await;
    harness.reset_counters();

    let offerer = harness.peer(100);

    tracing::info!("Step 2: Start network recovery while offerer signaling is disconnected");
    offerer
        .signaling_client
        .disconnect()
        .await
        .expect("test should disconnect offerer signaling");
    offerer
        .coordinator
        .begin_network_recovery("NetworkLost")
        .await;
    offerer
        .coordinator
        .restart_network_recovery_connections()
        .await;

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        harness.ice_restart_count(),
        0,
        "ICE restart must not send an offer while signaling is disconnected"
    );

    tracing::info!("Step 3: Reconnect signaling and issue repeated recovery resumes");
    offerer
        .signaling_client
        .connect_once()
        .await
        .expect("test should reconnect offerer signaling");
    for _ in 0..5 {
        offerer
            .coordinator
            .restart_network_recovery_connections()
            .await;
    }

    harness
        .wait_for_ice_restart_count(1, Duration::from_secs(3))
        .await;
    tokio::time::sleep(Duration::from_millis(2500)).await;

    assert_eq!(
        harness.ice_restart_count(),
        1,
        "repeated recovery resumes should wake the existing restart task, not send duplicate offers"
    );
}

#[tokio::test]
async fn test_network_recovery_guard_times_out_after_6s_and_closes_transport() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("Step 1: Establishing WebRTC connection 100 -> 200");
    harness.connect(100, 200).await;

    let target_id = harness.peer(200).id.clone();
    let dest = Dest::actor(target_id.clone());

    assert!(
        harness.peer(100).transport_manager.has_dest(&dest).await,
        "initial DestTransport should be cached before recovery guard timeout"
    );

    tracing::info!("Step 2: Mark the offerer peer as recovering via NetworkEvent guard");
    harness
        .peer(100)
        .coordinator
        .begin_network_recovery("test recovery timeout")
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let status = harness
        .peer(100)
        .coordinator
        .peer_recovery_status(&target_id)
        .await
        .expect("target should be guarded by network recovery");
    assert!(
        status.session_id > 0,
        "recovery guard should record the active WebRTC session id"
    );
    assert!(
        !status.is_timed_out(),
        "fresh network recovery guard should not be timed out"
    );

    tracing::info!("Step 3: Sends inside the 6s recovery window fail fast");
    let early = harness
        .peer(100)
        .spawn_request(200, "recovery-window-fast-fail", 30_000);
    expect_connection_recovering(early, "request inside recovery window").await;

    tracing::info!("Step 4: Age the guard beyond 6s and verify timeout cleanup");
    let expired_started_at = Instant::now() - Duration::from_secs(7);
    assert!(
        harness
            .peer(100)
            .coordinator
            .force_peer_recovery_started_at_for_test(&target_id, expired_started_at)
            .await,
        "test should be able to age the coordinator recovery guard"
    );

    let timed_out = harness
        .peer(100)
        .spawn_request(200, "recovery-window-timeout", 30_000);
    match tokio::time::timeout(Duration::from_secs(3), timed_out).await {
        Ok(Ok(Err(err))) => {
            let msg = err.to_string();
            assert!(
                msg.contains("Connection recovery timeout"),
                "expected recovery timeout error, got: {msg}"
            );
            assert!(
                msg.contains("timeout_ms=6000"),
                "timeout error should report the 6s recovery budget: {msg}"
            );
        }
        Ok(Ok(Ok(response))) => panic!(
            "timed-out recovery request unexpectedly succeeded with {} bytes",
            response.len()
        ),
        Ok(Err(err)) => panic!("timed-out recovery request task panicked: {err}"),
        Err(_) => panic!("timed-out recovery request did not fail fast"),
    }

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !harness.peer(100).transport_manager.has_dest(&dest).await,
        "recovery timeout should close and remove the stale DestTransport"
    );
    assert!(
        harness
            .peer(100)
            .coordinator
            .peer_recovery_status(&target_id)
            .await
            .is_none(),
        "recovery timeout should clear the coordinator guard"
    );
}

#[tokio::test]
async fn test_connection_closed_clears_recovery_guard_when_transport_already_removed() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    let target_id = harness.peer(200).id.clone();
    let synthetic_session_id = 77;

    tracing::info!("Step 1: Simulate a recovery guard for a session with no cached transport");
    harness
        .peer(100)
        .send_event(ConnectionEvent::IceRestartStarted {
            peer_id: target_id.clone(),
            session_id: synthetic_session_id,
        });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let blocked = harness
        .peer(100)
        .spawn_request(200, "synthetic-recovery-blocks-send", 5_000);
    expect_connection_recovering(blocked, "request before close event").await;

    tracing::info!("Step 2: Close the same session after the transport was already removed");
    harness
        .peer(100)
        .send_event(ConnectionEvent::ConnectionClosed {
            peer_id: target_id,
            session_id: synthetic_session_id,
        });
    tokio::time::sleep(Duration::from_millis(200)).await;

    tracing::info!("Step 3: A later send should create a fresh transport instead of waiting 15s");
    harness
        .connect_with_timeout(100, 200, Duration::from_secs(5))
        .await;
}

#[tokio::test]
async fn test_data_channel_on_close_cleans_webrtc_transport() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    let target_id = harness.peer(200).id.clone();
    let dest = Dest::actor(target_id.clone());
    let mut event_rx = harness.peer(100).subscribe_events();

    tracing::info!("Step 1: Establishing WebRTC connection 100 -> 200");
    harness.connect(100, 200).await;

    let session_id = wait_for_data_channel_opened(
        &mut event_rx,
        &target_id,
        PayloadType::RpcReliable,
        Duration::from_secs(5),
    )
    .await;
    tracing::info!(
        "Observed initial RpcReliable DataChannel for peer {:?}, session_id={}",
        target_id,
        session_id
    );

    assert!(
        harness.peer(100).transport_manager.has_dest(&dest).await,
        "initial DestTransport should be cached before DataChannel close"
    );

    tracing::info!("Step 2: Closing RpcReliable DataChannel to trigger on_close cleanup");
    let closed_session_id = harness
        .peer(100)
        .coordinator
        .close_data_channel_for_test(&target_id, PayloadType::RpcReliable)
        .await
        .expect("active RpcReliable DataChannel should be closable");
    assert_eq!(
        closed_session_id, session_id,
        "test should close the same WebRTC session observed during connect"
    );

    let closed_payload_type = wait_for_data_channel_close_chain(
        &mut event_rx,
        &target_id,
        session_id,
        Duration::from_secs(10),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        !harness
            .peer(100)
            .coordinator
            .has_open_data_channel_for_test(&target_id)
            .await
            .expect("DataChannel state should be queryable after close"),
        "DataChannel on_close should leave no open DataChannel on the closed WebRTC session"
    );
    assert!(
        !harness.peer(100).transport_manager.has_dest(&dest).await,
        "DataChannel on_close should lead to ConnectionClosed and remove stale DestTransport"
    );

    tracing::info!(
        "DataChannel close chain cleaned transport for peer {:?}, session_id={}, first_closed_payload_type={:?}",
        target_id,
        session_id,
        closed_payload_type
    );
}

// ==================== Test 1: Two-peer disconnect/reconnect with NetworkEvent ====================

/// Test: disconnect two peers via VNet + signaling pause,
/// simulate NetworkEvent::Available (retry_failed_connections),
/// verify the connection is actually recovered by sending a message through the gate.
#[tokio::test]
async fn test_two_peer_disconnect_reconnect() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("🔗 Step 1: Establishing connection 100 → 200...");
    harness.connect(100, 200).await;

    // Record baseline
    harness.reset_counters();

    tracing::info!("🔴 Step 2: Simulating full network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    // Wait for ICE to detect disconnection
    tracing::info!("⏳ Waiting for ICE disconnection detection...");
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Verify ICE restart was triggered (even though it can't succeed — signaling is down)
    let post_disconnect_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart count during outage: {}",
        post_disconnect_count
    );

    tracing::info!("🟢 Step 3: Restoring network (VNet + signaling)...");
    harness.simulate_reconnect();

    // Step 4: Simulate NetworkEvent::Available → triggers retry_failed_connections()
    // This is what happens in production when the platform layer detects network recovery
    tracing::info!("📱 Step 4: Triggering NetworkEvent::Available (retry_failed_connections)...");
    let start = tokio::time::Instant::now();
    harness.peer(100).retry_failed().await;

    // Wait for ICE restart to complete on the recovered network
    tracing::info!("⏳ Waiting for ICE restart to complete...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    let recovery_time = start.elapsed();
    tracing::info!(
        "📊 Recovery time (from NetworkEvent::Available): {:?}",
        recovery_time
    );

    // Step 5: Verify connection is ACTUALLY recovered by sending a message
    tracing::info!("📤 Step 5: Verifying connection recovery via gate message...");
    let peer_a = harness.peer(100);
    let request_handle = peer_a.spawn_request(200, "reconnect_verify_1", 10000);

    match tokio::time::timeout(Duration::from_secs(10), request_handle).await {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Connection recovered! Response: {} bytes, total recovery: {:?}",
                response.len(),
                start.elapsed()
            );
        }
        Ok(Ok(Err(e))) => {
            panic!("❌ Connection NOT recovered — request failed: {}", e);
        }
        Ok(Err(e)) => panic!("Request task panicked: {}", e),
        Err(_) => panic!("❌ Connection NOT recovered — request timed out after 10s"),
    }

    tracing::info!("✅ test_two_peer_disconnect_reconnect passed!");
}

#[tokio::test]
async fn test_lost_available_type_changed_batch_restores_webrtc_end_to_end() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("🔗 Step 1: Establishing connection 100 → 200...");
    harness.connect(100, 200).await;

    harness.reset_counters();

    tracing::info!("🔴 Step 2: Simulating full network outage (VNet + signaling)...");
    harness.simulate_disconnect();
    tokio::time::sleep(Duration::from_secs(8)).await;

    tracing::info!("🟢 Step 3: Restoring network (VNet + signaling)...");
    harness.simulate_reconnect();

    let events = vec![
        NetworkEvent::Lost,
        NetworkEvent::Available,
        NetworkEvent::TypeChanged {
            is_wifi: true,
            is_cellular: false,
        },
    ];
    assert_eq!(
        select_network_recovery_action(&events),
        NetworkRecoveryAction::Restore
    );

    tracing::info!("📱 Step 4: Processing Lost -> Available -> TypeChanged as one batch...");
    let results = process_network_event_batch(events, harness.peer(100).network_processor()).await;
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| result.success));

    let restart_count = harness
        .wait_for_ice_restart_count(1, Duration::from_secs(10))
        .await;
    tracing::info!(
        "📊 ICE restart count after batched restore: {}",
        restart_count
    );

    tracing::info!("📤 Step 5: Verifying WebRTC recovery via gate message...");
    let response = expect_request_eventually_ok(
        &harness,
        100,
        200,
        "batched_network_restore_verify",
        Duration::from_secs(20),
        2_000,
    )
    .await;
    tracing::info!(
        "✅ WebRTC recovered after batched network events: {} bytes",
        response.len()
    );
}

#[tokio::test]
async fn test_cleanup_available_type_changed_batch_rebuilds_webrtc_end_to_end() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("🔗 Step 1: Establishing connection 100 → 200...");
    harness.connect(100, 200).await;

    harness.reset_counters();

    let events = vec![
        NetworkEvent::CleanupConnections,
        NetworkEvent::Available,
        NetworkEvent::TypeChanged {
            is_wifi: true,
            is_cellular: false,
        },
    ];
    assert_eq!(
        select_network_recovery_action(&events),
        NetworkRecoveryAction::CleanupConnectionsCompat
    );

    tracing::info!(
        "📱 Step 2: Processing CleanupConnections -> Available -> TypeChanged as one batch..."
    );
    let results = process_network_event_batch(events, harness.peer(100).network_processor()).await;
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| result.success));

    tracing::info!("📤 Step 3: Verifying WebRTC can rebuild via gate message...");
    let request_handle =
        harness
            .peer(100)
            .spawn_request(200, "cleanup_batch_rebuild_verify", 15000);

    match tokio::time::timeout(Duration::from_secs(15), request_handle).await {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ WebRTC rebuilt after cleanup batch: {} bytes",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => panic!("❌ WebRTC not rebuilt — request failed: {}", e),
        Ok(Err(e)) => panic!("Request task panicked: {}", e),
        Err(_) => panic!("❌ WebRTC not rebuilt — request timed out after 15s"),
    }
}

// ==================== Test 2: Offerer recovery latency ====================

/// Test: offerer-triggered recovery after network outage.
///
/// Topology: peer 200 → peer 100 (offerer, echo responder on 100)
///
/// Flow:
/// 1. Establish connection
/// 2. Full network outage (VNet + signaling) for 8s
///    → ICE Disconnected → auto-restart triggered on offerer (peer 100)
///    → First attempt fails (signaling blocked) → enters backoff
/// 3. Unblock network
/// 4. Offerer (peer 100) calls `retry_failed_connections()` (simulating NetworkEvent::Available)
///    → `restart_ice()` but already inflight → no-op (dedup check)
/// 5. Measure time from unblock to message delivery
///
/// Key observation: `retry_failed()` on offerer is a no-op because
/// `do_ice_restart_inner` is already running. Recovery depends entirely on
/// the existing backoff timer expiring and retrying.
#[tokio::test]
async fn test_offerer_recovery_latency() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await; // offerer (first peer → offerer VNet)
    harness.add_peer(200).await; // answerer

    tracing::info!("🔗 Step 1: Establishing connection 200 → 100...");
    tracing::info!("   Peer 100 = offerer (echo responder)");
    tracing::info!("   Peer 200 = answerer (message sender)");
    harness.connect(200, 100).await;

    harness.reset_counters();

    // === Step 2: Short outage — connection stays in peers map ===
    tracing::info!("🔴 Step 2: Full network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    // Wait for ICE Disconnected → auto-restart → first attempt fails → enters backoff
    tracing::info!("⏳ Waiting 8s for auto-restart to enter backoff...");
    tokio::time::sleep(Duration::from_secs(8)).await;

    let outage_restart_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart attempts during outage: {} (all failed — signaling blocked)",
        outage_restart_count
    );

    // === Step 3: Unblock network — start measuring ===
    tracing::info!("🟢 Step 3: Restoring network — timer starts NOW");
    let recovery_start = std::time::Instant::now();
    harness.simulate_reconnect();

    // === Step 4: Offerer calls retry_failed (simulating NetworkEvent::Available) ===
    tracing::info!("📱 Step 4: Offerer (100) calls retry_failed_connections()...");
    tracing::info!("   → restart_ice() will find restart already inflight → no-op");
    harness.peer(100).retry_failed().await;

    // === Step 5: Wait for recovery and send message ===
    tracing::info!("📤 Step 5: Sending message 200→100 to verify recovery...");
    let response = expect_request_eventually_ok(
        &harness,
        200,
        100,
        "offerer_recovery",
        Duration::from_secs(30),
        2_000,
    )
    .await;
    let e2e_latency = recovery_start.elapsed();
    tracing::info!(
        "✅ Offerer recovery succeeded! Response: {} bytes",
        response.len()
    );

    let total_restart_count = harness.ice_restart_count();

    tracing::info!("╔══════════════════════════════════════════════════════╗");
    tracing::info!("║   Offerer Recovery Summary                          ║");
    tracing::info!("╠══════════════════════════════════════════════════════╣");
    tracing::info!("║ E2E recovery latency: {:?}", e2e_latency);
    tracing::info!("║   (from network unblock to message response)");
    tracing::info!(
        "║ ICE restart attempts: {} during outage, {} total",
        outage_restart_count,
        total_restart_count
    );
    tracing::info!("║ Note: retry_failed() on offerer = no-op (restart");
    tracing::info!("║   already inflight, dedup check blocks it)");
    tracing::info!("╚══════════════════════════════════════════════════════╝");

    tracing::info!("✅ test_offerer_recovery_latency passed!");
}

// ==================== Test 3: Answerer recovery latency ====================

/// Test: answerer-triggered recovery after network outage (Plan A).
///
/// Topology: peer 200 → peer 100 (offerer, echo responder on 100)
///
/// Same setup as offerer test, BUT:
/// 4. **Answerer (peer 200)** calls `retry_failed_connections()` instead
///    → `restart_ice()` → `!is_offerer` → sends IceRestartRequest to Offerer
/// 5. Offerer receives IceRestartRequest → `notify_one()` wakes backoff
///    → immediate ICE restart retry → FASTER recovery
#[tokio::test]
async fn test_answerer_recovery_latency() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await; // offerer (first peer → offerer VNet)
    harness.add_peer(200).await; // answerer

    tracing::info!("🔗 Step 1: Establishing connection 200 → 100...");
    tracing::info!("   Peer 100 = offerer (echo responder)");
    tracing::info!("   Peer 200 = answerer (message sender, focus of this test)");
    harness.connect(200, 100).await;

    harness.reset_counters();

    // === Step 2: Short outage — connection stays in peers map ===
    tracing::info!("🔴 Step 2: Full network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    tracing::info!("⏳ Waiting 8s for auto-restart to enter backoff...");
    tokio::time::sleep(Duration::from_secs(8)).await;

    let outage_restart_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart attempts during outage: {} (all failed — signaling blocked)",
        outage_restart_count
    );

    // === Step 3: Unblock network — start measuring ===
    tracing::info!("🟢 Step 3: Restoring network — timer starts NOW");
    let recovery_start = std::time::Instant::now();
    harness.simulate_reconnect();

    // === Step 4: ANSWERER calls retry_failed (simulating NetworkEvent::Available) ===
    tracing::info!("📱 Step 4: Answerer (200) calls retry_failed_connections()...");
    tracing::info!("   → restart_ice() → !is_offerer → sends IceRestartRequest to Offerer");
    harness.peer(200).retry_failed().await;

    // === Step 5: Wait for recovery and send message ===
    tracing::info!("📤 Step 5: Sending message 200→100 to verify recovery...");
    let response = expect_request_eventually_ok(
        &harness,
        200,
        100,
        "answerer_recovery",
        Duration::from_secs(30),
        2_000,
    )
    .await;
    let e2e_latency = recovery_start.elapsed();
    tracing::info!(
        "✅ Answerer (200) recovered! Response: {} bytes",
        response.len()
    );

    let total_restart_count = harness.ice_restart_count();

    tracing::info!("╔══════════════════════════════════════════════════════╗");
    tracing::info!("║   Answerer Recovery Summary                         ║");
    tracing::info!("╠══════════════════════════════════════════════════════╣");
    tracing::info!("║ E2E recovery latency: {:?}", e2e_latency);
    tracing::info!("║   (from network unblock to message response)");
    tracing::info!(
        "║ ICE restart attempts: {} during outage, {} total",
        outage_restart_count,
        total_restart_count
    );
    tracing::info!("║ Plan A: retry_failed() on answerer -> IceRestartRequest");
    tracing::info!("║   → Offerer wakes backoff → immediate retry");
    tracing::info!("╚══════════════════════════════════════════════════════╝");

    tracing::info!("✅ test_answerer_recovery_latency completed!");
}

// ==================== Repro: NetworkEvent returns before WebRTC is usable ====================

/// Reproduces the mobile 5G -> WiFi failure mode:
///
/// 1. The client-side NetworkEvent path returns success after it reconnects
///    signaling and starts/retries WebRTC recovery.
/// 2. That success does not mean the reliable DataChannel is usable yet.
/// 3. RPCs sent immediately after the event now fail fast with
///    `Connection recovering` before they enter pending_requests.
/// 4. A later retry succeeds once UDP/signaling are restored.
#[tokio::test]
#[ignore = "slow VNet recovery regression test"]
async fn repro_network_event_returns_before_webrtc_ready_causing_early_rpc_timeouts() {
    init_tracing();

    const CLIENT: u64 = 100;
    const SERVER: u64 = 200;

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(CLIENT).await;
    harness.add_peer(SERVER).await;

    let server_id = harness.peer(SERVER).id.clone();
    let mut client_events = harness.peer(CLIENT).subscribe_events();

    tracing::info!("Step 1: Establish client -> server WebRTC RPC path");
    harness.connect(CLIENT, SERVER).await;
    harness.reset_counters();

    tracing::info!(
        "Step 2: Simulate network switch window: UDP blocked, signaling forwarding paused"
    );
    harness
        .vnet
        .as_ref()
        .expect("test requires VNet")
        .block_network();
    harness.server.pause_forwarding();

    let (session_id, state) = wait_for_peer_state(
        &mut client_events,
        &server_id,
        &[ConnectionState::Disconnected, ConnectionState::Failed],
        Duration::from_secs(12),
    )
    .await;
    tracing::info!(
        "Client observed server session {} enter {:?}",
        session_id,
        state
    );

    tracing::info!("Step 3: Run the same NetworkEvent processor used by mobile bindings");
    assert!(
        harness.peer(CLIENT).signaling_client.is_connected(),
        "client signaling should be connected before NetworkEvent closes it"
    );
    let processor = DefaultNetworkEventProcessor::new(
        harness.peer(CLIENT).signaling_client.clone(),
        Some(harness.peer(CLIENT).coordinator.clone()),
    );
    let event_started = std::time::Instant::now();
    processor
        .process_network_type_changed(true, false)
        .await
        .expect("NetworkEvent::TypeChanged should report success");
    let event_elapsed = event_started.elapsed();
    tracing::info!(
        "NetworkEvent::TypeChanged returned in {:?}; ICE restart offers observed={}",
        event_elapsed,
        harness.ice_restart_count()
    );
    wait_for_signaling_reconnect(&harness, 1, 1, Duration::from_secs(2)).await;
    assert!(
        harness.peer(CLIENT).signaling_client.is_connected(),
        "client signaling should be reconnected after NetworkEvent returns"
    );
    assert!(
        event_elapsed < Duration::from_secs(3),
        "NetworkEvent returned too slowly for this repro: {:?}",
        event_elapsed
    );

    tracing::info!("Step 4: Send two RPCs immediately after NetworkEvent returns");
    let client = harness.peer(CLIENT);
    let early_1 = client.spawn_request(SERVER, "network-event-early-timeout-1", 500);
    let early_2 = client.spawn_request(SERVER, "network-event-early-timeout-2", 800);

    expect_connection_recovering(early_1, "first immediate RPC").await;
    expect_connection_recovering(early_2, "second immediate RPC").await;

    tracing::info!("Step 5: Finish network recovery; a later retry should succeed");
    harness.simulate_reconnect();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let request_id = format!("network-event-late-success-{attempt}");
        let late_success = harness
            .peer(CLIENT)
            .spawn_request(SERVER, &request_id, 2_000);

        match tokio::time::timeout(Duration::from_secs(3), late_success).await {
            Ok(Ok(Ok(response))) => {
                tracing::info!(
                    "Retry after delayed recovery received {} bytes on attempt {}",
                    response.len(),
                    attempt
                );
                assert_eq!(&response[..], b"pong");
                break;
            }
            Ok(Ok(Err(err))) => {
                let msg = err.to_string();
                if tokio::time::Instant::now() >= deadline {
                    panic!("retry after recovery should eventually succeed, last error: {msg}");
                }
                assert!(
                    msg.contains("Connection recovering")
                        || msg.contains("Request timeout")
                        || msg.contains("Connection"),
                    "unexpected retry error while waiting for recovery: {msg}"
                );
            }
            Ok(Err(err)) => panic!("retry task panicked: {err}"),
            Err(_) if tokio::time::Instant::now() < deadline => {}
            Err(_) => panic!("retry did not complete after network recovery"),
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

// ==================== Regression: foreground cleanup overlaps a new send ====================

/// Exercises the slow-send relative timeline from the mobile log:
///
/// - T+0ms: app returns foreground.
/// - T+403ms: cleanup starts and closes the old client-side WebRTC peers.
/// - T+1972ms: user sends a new RPC.
/// - T+2405ms: cleanup disconnects/rebuilds the signaling WebSocket.
///
/// Before the cleanup barrier, the fresh WebRTC negotiation could be
/// interrupted after SDP exchange but before usable ICE candidate exchange,
/// then wait for the 10s connection-establishment timeout plus the 5s factory
/// retry backoff. With the fix, the send waits for cleanup and avoids that slow
/// retry path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "log-timeline foreground cleanup/request-overlap regression"]
async fn regression_log_timeline_foreground_cleanup_waits_before_new_send() {
    init_tracing();

    const SERVER: u64 = 100;
    const CLIENT: u64 = 200;
    const CLEANUP_START_MS: u64 = 403;
    const USER_SEND_MS: u64 = 1_972;
    const CLEANUP_SIGNALING_DISCONNECT_MS: u64 = 2_405;

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(SERVER).await;
    harness.add_peer(CLIENT).await;

    tracing::info!("Step 1: Establish the pre-background client -> server path");
    harness
        .connect_with_timeout(CLIENT, SERVER, Duration::from_secs(30))
        .await;
    harness.reset_counters();

    tracing::info!("Step 2: Start the foreground cleanup timeline");
    assert!(
        harness.peer(CLIENT).signaling_client.is_connected(),
        "client signaling should be connected before foreground cleanup"
    );

    let foreground_started = tokio::time::Instant::now();
    let cleanup_coordinator = harness.peer(CLIENT).coordinator.clone();
    let cleanup_signaling = harness.peer(CLIENT).signaling_client.clone();
    let cleanup_task = tokio::spawn(async move {
        tokio::time::sleep_until(foreground_started + Duration::from_millis(CLEANUP_START_MS))
            .await;
        let _cleanup_guard = cleanup_coordinator.cleanup_guard();

        cleanup_coordinator.clear_pending_restarts().await;
        cleanup_coordinator
            .close_all_peers()
            .await
            .map_err(|err| err.to_string())?;

        tokio::time::sleep_until(
            foreground_started + Duration::from_millis(CLEANUP_SIGNALING_DISCONNECT_MS),
        )
        .await;
        if cleanup_signaling.is_connected() {
            cleanup_signaling
                .disconnect()
                .await
                .map_err(|err| err.to_string())?;
        }
        cleanup_signaling
            .connect_once()
            .await
            .map_err(|err| err.to_string())?;

        Ok::<(), String>(())
    });

    tokio::time::sleep_until(foreground_started + Duration::from_millis(USER_SEND_MS)).await;

    tracing::info!("Step 3: User sends according to the log timeline");
    harness.server.drop_next_ice_candidates_for(
        2,
        Duration::from_millis(CLEANUP_SIGNALING_DISCONNECT_MS - USER_SEND_MS),
    );
    let send_started = Instant::now();
    let request =
        harness
            .peer(CLIENT)
            .spawn_request(SERVER, "log-timeline-foreground-overlap-send", 30_000);

    tokio::time::sleep_until(
        foreground_started + Duration::from_millis(CLEANUP_SIGNALING_DISCONNECT_MS),
    )
    .await;

    let response = match tokio::time::timeout(Duration::from_secs(35), request).await {
        Ok(Ok(Ok(response))) => response,
        Ok(Ok(Err(err))) => panic!("overlapped foreground send failed: {err}"),
        Ok(Err(err)) => panic!("overlapped foreground send task panicked: {err}"),
        Err(_) => panic!("overlapped foreground send did not complete within 35s"),
    };
    let send_elapsed = send_started.elapsed();

    let cleanup_result = cleanup_task.await.expect("cleanup task panicked");
    assert!(
        cleanup_result.is_ok(),
        "foreground cleanup should complete: {:?}",
        cleanup_result
    );

    tracing::info!(
        "Log-timeline foreground-overlap send completed in {:?} with {} response bytes",
        send_elapsed,
        response.len()
    );

    assert_eq!(&response[..], b"pong");
    assert!(
        send_elapsed < Duration::from_secs(8),
        "send should wait for cleanup instead of hitting the 10s timeout + 5s retry path, got {:?}",
        send_elapsed
    );
}
