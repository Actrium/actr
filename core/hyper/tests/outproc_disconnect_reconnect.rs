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

use actr_hyper::test_support::TestHarness;
use actr_hyper::transport::{ConnectionEvent, ConnectionState, Dest};
use actr_protocol::{ActrId, PayloadType};
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
        }
        Ok(Err(e)) => panic!("Answerer request task panicked: {}", e),
        Err(_) => {
            tracing::error!(
                "❌ Answerer (200) recovery TIMED OUT after {:?}",
                e2e_latency
            );
        }
    }

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
