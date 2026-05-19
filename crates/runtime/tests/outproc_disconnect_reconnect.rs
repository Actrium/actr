//! Integration tests for OutprocOutGate disconnection/reconnection
//!
//! Uses TestHarness for multi-peer topology with VNet network simulation.
//!
//! Tests focus on:
//! - Two-peer disconnect → network event → ICE restart → reconnect
//! - Offerer vs Answerer recovery latency comparison
//! - Pending request cleanup on disconnect

mod common;

use actr_protocol::{ActrId, PayloadType};
use actr_runtime::lifecycle::{
    DefaultNetworkEventProcessor, NetworkEvent, NetworkEventProcessor, NetworkRecoveryAction,
    process_network_event_batch, select_network_recovery_action,
};
use actr_runtime::transport::{ConnectionEvent, ConnectionState, DataLane, Dest};
use common::TestHarness;
use std::time::Duration;

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
            "timed out waiting for {:?} DataChannelOpened for peer {}",
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
                    "timed out waiting for {:?} DataChannelOpened for peer {}",
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
                    "Observed DataChannelClosed for peer {}, session_id={}, payload_type={:?}",
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
                    "Observed ConnectionClosed for peer {}, session_id={}",
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
                    "Observed PeerConnection Closed for peer {}, session_id={}",
                    peer_id,
                    session_id
                );
                saw_peer_connection_closed = true;
            }
            _ => {}
        }

        if closed_payload_type.is_some() && saw_peer_connection_closed && saw_connection_closed {
            return closed_payload_type.expect("closed payload type should be set");
        }
    }

    panic!(
        "timed out waiting for DataChannelClosed -> PeerConnection Closed -> ConnectionClosed chain for peer {}, session_id={}, saw_data_channel_closed={}, saw_peer_connection_closed={}, saw_connection_closed={}",
        peer_id,
        session_id,
        closed_payload_type.is_some(),
        saw_peer_connection_closed,
        saw_connection_closed
    );
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
        "Observed initial RpcReliable DataChannel for peer {}, session_id={}",
        target_id,
        session_id
    );

    assert!(
        harness.peer(100).transport_manager.has_dest(&dest).await,
        "initial DestTransport should be cached before DataChannel close"
    );

    tracing::info!("Step 2: Closing RpcReliable DataChannel to trigger on_close cleanup");
    let webrtc_conn = harness
        .peer(100)
        .coordinator
        .create_connection(&dest, None)
        .await
        .expect("active WebRTC connection should be reusable");
    assert_eq!(
        webrtc_conn.session_id(),
        session_id,
        "test should close the same WebRTC session observed during connect"
    );

    let lane = webrtc_conn
        .get_lane(PayloadType::RpcReliable)
        .await
        .expect("RpcReliable lane should be cached/open before close");
    match lane {
        DataLane::WebRtcDataChannel { data_channel, .. } => {
            data_channel
                .close()
                .await
                .expect("closing RTCDataChannel should trigger on_close");
        }
        _ => panic!("RpcReliable lane should use WebRTC DataChannel"),
    }

    let closed_payload_type = wait_for_data_channel_close_chain(
        &mut event_rx,
        &target_id,
        session_id,
        Duration::from_secs(10),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        !webrtc_conn.has_open_data_channel().await,
        "DataChannel on_close should leave no open DataChannel on the closed WebRTC session"
    );
    assert!(
        !harness.peer(100).transport_manager.has_dest(&dest).await,
        "DataChannel on_close should lead to ConnectionClosed and remove stale DestTransport"
    );

    tracing::info!(
        "DataChannel close chain cleaned transport for peer {}, session_id={}, first_closed_payload_type={:?}",
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
    let request_handle =
        harness
            .peer(100)
            .spawn_request(200, "batched_network_restore_verify", 10000);

    match tokio::time::timeout(Duration::from_secs(10), request_handle).await {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ WebRTC recovered after batched network events: {} bytes",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => panic!("❌ WebRTC not recovered — request failed: {}", e),
        Ok(Err(e)) => panic!("Request task panicked: {}", e),
        Err(_) => panic!("❌ WebRTC not recovered — request timed out after 10s"),
    }
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
        "📱 Step 2: Processing cleanup_connections -> Available -> TypeChanged as one batch..."
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

/// Test: offerer recovery after long network outage.
///
/// Topology: peer 200 sends to peer 100 (offerer, echo responder)
///
/// Recovery measurement (event-driven):
/// - Timer starts at `simulate_reconnect()` (network unblock)
/// - Send message to trigger new connection establishment
/// - Measure time until message response (end-to-end recovery)
///
/// This measures the REAL recovery latency — from network restoration
/// to successful message delivery — not the connection_factory backoff.
#[tokio::test]
async fn test_offerer_recovery_latency() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("🔗 Step 1: Establishing connection 200 → 100...");
    tracing::info!("   Peer 100 = offerer (echo responder)");
    tracing::info!("   Peer 200 = answerer (message sender)");
    harness.connect(200, 100).await;

    harness.reset_counters();

    tracing::info!("🔴 Step 2: Simulating long network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    // Wait long enough for ICE restart retries to exhaust and peer to be dropped
    tracing::info!("⏳ Waiting 15s for connection to fully fail...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    let outage_restart_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart attempts during outage: {} (all failed — signaling was down)",
        outage_restart_count
    );

    // --- Recovery: start timer from network unblock ---
    tracing::info!("🟢 Step 3: Restoring network — timer starts NOW");
    let recovery_start = std::time::Instant::now();
    harness.simulate_reconnect();

    // Send message to trigger new connection (200→100, echo responder on 100)
    tracing::info!("📱 Step 4: Sending message 200→100 to trigger new connection...");
    let peer_200 = harness.peer(200);
    let msg_handle = peer_200.spawn_request(100, "offerer_recovery", 30000);

    let msg_result = tokio::time::timeout(Duration::from_secs(30), msg_handle).await;
    let e2e_latency = recovery_start.elapsed();

    match msg_result {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Offerer recovery succeeded! Response: {} bytes",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => {
            panic!(
                "❌ Offerer recovery FAILED: {} (e2e latency: {:?})",
                e, e2e_latency
            );
        }
        Ok(Err(e)) => panic!("Offerer request task panicked: {}", e),
        Err(_) => {
            panic!("❌ Offerer recovery TIMED OUT after {:?}", e2e_latency);
        }
    }

    tracing::info!("╔══════════════════════════════════════════╗");
    tracing::info!("║   Offerer Recovery Summary               ║");
    tracing::info!("╠══════════════════════════════════════════╣");
    tracing::info!("║ E2E recovery latency: {:?}", e2e_latency);
    tracing::info!("║   (from network unblock to message response)");
    tracing::info!("║ Outage ICE restart attempts: {}", outage_restart_count);
    tracing::info!("╚══════════════════════════════════════════╝");

    tracing::info!("✅ test_offerer_recovery_latency passed!");
}

// ==================== Test 3: Answerer recovery latency ====================

/// Test: answerer recovery after long network outage.
///
/// Same topology and flow as offerer test — both use the same message direction
/// (200→100), so the difference is purely observational.
///
/// After long outage, the old connection is dropped. A new message triggers
/// a fresh RoleNegotiation. Recovery measurement starts at network unblock.
#[tokio::test]
async fn test_answerer_recovery_latency() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("🔗 Step 1: Establishing connection 200 → 100...");
    tracing::info!("   Peer 100 = offerer (echo responder)");
    tracing::info!("   Peer 200 = answerer (message sender, focus of this test)");
    harness.connect(200, 100).await;

    harness.reset_counters();

    tracing::info!("🔴 Step 2: Simulating long network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    tracing::info!("⏳ Waiting 15s for connection to fully fail...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    let outage_restart_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart attempts during outage: {} (all failed — signaling was down)",
        outage_restart_count
    );

    // --- Recovery: start timer from network unblock ---
    tracing::info!("🟢 Step 3: Restoring network — timer starts NOW");
    let recovery_start = std::time::Instant::now();
    harness.simulate_reconnect();

    // Send message from answerer side (200→100, echo responder on 100)
    tracing::info!(
        "📱 Step 4: Answerer (200) sending message 200→100 to trigger new connection..."
    );
    let peer_200 = harness.peer(200);
    let msg_handle = peer_200.spawn_request(100, "answerer_recovery", 30000);

    let msg_result = tokio::time::timeout(Duration::from_secs(30), msg_handle).await;
    let e2e_latency = recovery_start.elapsed();

    match msg_result {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Answerer (200) recovered! Response: {} bytes",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => {
            tracing::error!(
                "❌ Answerer (200) recovery FAILED: {} (e2e latency: {:?})",
                e,
                e2e_latency
            );
            tracing::error!("   This may indicate role-based recovery differences");
        }
        Ok(Err(e)) => panic!("Answerer request task panicked: {}", e),
        Err(_) => {
            tracing::error!(
                "❌ Answerer (200) recovery TIMED OUT after {:?}",
                e2e_latency
            );
            tracing::error!("   This may indicate role-based recovery differences");
        }
    }

    tracing::info!("╔══════════════════════════════════════════╗");
    tracing::info!("║   Answerer Recovery Summary              ║");
    tracing::info!("╠══════════════════════════════════════════╣");
    tracing::info!("║ E2E recovery latency: {:?}", e2e_latency);
    tracing::info!("║   (from network unblock to message response)");
    tracing::info!("║ Outage ICE restart attempts: {}", outage_restart_count);
    tracing::info!("╚══════════════════════════════════════════╝");

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
///
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
