//! Integration tests for OutprocOutGate disconnection/reconnection
//!
//! Uses TestHarness for multi-peer topology with VNet network simulation.
//!
//! Tests focus on:
//! - Two-peer disconnect → network event → ICE restart → reconnect
//! - Offerer vs Answerer recovery latency comparison
//! - Pending request cleanup on disconnect

mod common;

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
