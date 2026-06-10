//! Regression test for: response send blocked by recovery guard during ICE restart.
//!
//! When a server (offerer) initiates ICE restart upon receiving an IceRestartRequest
//! from a mobile answerer, the coordinator sets a network recovery status on its
//! `network_recovering_peers` map. If the server tries to send a response (e.g. echo)
//! while the recovery is active, `preflight_send` returns
//! Unavailable("Connection recovering: ...") and the response is dropped — no retry.
//!
//! ICE restart typically completes in ~200ms, but the guard window is enough for
//! an in-flight response to be lost. This test reliably reproduces the issue by
//! calling `begin_network_recovery` on the server's coordinator to set the
//! authoritative recovery status (not just the PeerGate's local guard), then
//! verifying the response behavior.

use actr_framework::Bytes;
use actr_hyper::test_support::TestHarness;
use actr_hyper::transport::ConnectionEvent;
use actr_hyper::wire::webrtc::WebRtcCoordinator;
use actr_protocol::{ActrId, PayloadType, RpcEnvelope};
use std::time::Duration;

const SERVER: u64 = 100;
const MOBILE_ANSWERER: u64 = 200;

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();
}

/// Get the current session_id for a peer's connection to another peer.
async fn get_peer_session_id(coordinator: &WebRtcCoordinator, peer_id: &ActrId) -> Option<u64> {
    coordinator.get_peer_session_id(peer_id).await
}

/// Test: send_message is blocked by recovery guard set via begin_network_recovery,
/// but send_response_with_recovery_retry waits for the guard to clear and then succeeds.
///
/// This test calls `begin_network_recovery` on the server's coordinator to set the
/// authoritative recovery status in `network_recovering_peers`, which `preflight_send`
/// checks first. This is the same mechanism that fires in production when a network
/// change triggers ICE restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_response_retry_through_ice_restart_recovery_guard() {
    init_tracing();

    // 1. Setup: mobile (answerer, serial=200) + server (offerer, serial=100)
    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(SERVER).await;
    harness.add_peer(MOBILE_ANSWERER).await;
    harness.connect(MOBILE_ANSWERER, SERVER).await;

    let server_gate = harness.peer(SERVER).gate.clone();
    let server_coordinator = harness.peer(SERVER).coordinator.clone();
    let mobile_id = harness.peer(MOBILE_ANSWERER).id.clone();

    // 2. Get the current session_id (needed to verify recovery status)
    let session_id = get_peer_session_id(&server_coordinator, &mobile_id)
        .await
        .expect("server should have a session with mobile");

    // 3. Call begin_network_recovery on the server's coordinator.
    //    This sets the authoritative recovery status in `network_recovering_peers`,
    //    which is the FIRST check in preflight_send (step 1). Unlike injecting
    //    IceRestartStarted via send_event (which only sets PeerGate's local guard
    //    that gets self-healed), begin_network_recovery makes preflight_send
    //    reliably return a recovering error.
    let recovered_peers = server_coordinator
        .begin_network_recovery("ice restart started")
        .await;
    assert!(
        recovered_peers.iter().any(|p| p == &mobile_id),
        "mobile should be marked as recovering, got: {:?}",
        recovered_peers
    );

    // 4. Verify: send_message is blocked by the recovery guard.
    //    Poll briefly to confirm the guard is active.
    let probe = RpcEnvelope {
        request_id: "guard_probe".to_string(),
        route_key: "response".to_string(),
        payload: Some(Bytes::from("probe")),
        timeout_ms: 0,
        ..Default::default()
    };
    let probe_result = server_gate.send_message(&mobile_id, probe).await;
    assert!(
        probe_result.is_err(),
        "send_message during recovery guard should fail, got: {:?}",
        probe_result
    );
    let err_msg = probe_result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Connection recovering"),
        "error should mention recovering, got: {}",
        err_msg
    );

    // 5. Verify: send_response_with_recovery_retry is blocked initially,
    //    but we schedule a guard-clear event in 500ms.
    //    The retry method should wait for the guard to clear and then succeed.
    let mobile_id_clone = mobile_id.clone();
    let server_coordinator_clone = server_coordinator.clone();
    let clear_guard_handle = tokio::spawn(async move {
        // Wait 500ms then simulate ICE restart completion to clear the guard.
        // IceRestartCompleted is processed by both the coordinator's event loop
        // (which clears network_recovering_peers if the peer is sendable) and
        // the PeerGate's event listener (which clears the local guard).
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ =
            server_coordinator_clone
                .event_sender()
                .send(ConnectionEvent::IceRestartCompleted {
                    peer_id: mobile_id_clone,
                    session_id,
                    success: true,
                });
    });

    let retry_response = RpcEnvelope {
        request_id: "test_retry_waits_for_guard".to_string(),
        route_key: "response".to_string(),
        payload: Some(Bytes::from("should_succeed_after_wait")),
        timeout_ms: 0,
        ..Default::default()
    };
    let retry_result = tokio::time::timeout(
        Duration::from_secs(5),
        server_gate.send_response_with_recovery_retry(
            &mobile_id,
            PayloadType::RpcReliable,
            retry_response,
        ),
    )
    .await;

    // Ensure the clear-guard task completes
    let _ = clear_guard_handle.await;

    assert!(
        retry_result.is_ok(),
        "send_response_with_recovery_retry should not timeout waiting for guard to clear"
    );
    let inner_result = retry_result.unwrap();
    assert!(
        inner_result.is_ok(),
        "send_response_with_recovery_retry should succeed after guard clears, got: {:?}",
        inner_result
    );
}

/// End-to-end test: request/response through echo responder during recovery guard.
///
/// The echo responder now uses send_response_with_recovery_retry, so responses
/// that hit the recovery guard will wait and retry instead of being dropped.
///
/// This test verifies the full round-trip: mobile sends request → server echo
/// responder receives it → tries to respond → hits guard → retries → guard
/// clears → response delivered → mobile receives response.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_echo_response_retries_through_ice_restart_recovery_guard() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(SERVER).await;
    harness.add_peer(MOBILE_ANSWERER).await;
    harness.connect(MOBILE_ANSWERER, SERVER).await;

    // Start echo responder on server, response receiver on mobile
    let _server_echo = harness.peer(SERVER).start_echo_responder("server_echo_e2e");
    let _mobile_rx = harness
        .peer(MOBILE_ANSWERER)
        .start_response_receiver("mobile_rx_e2e");

    let server_coordinator = harness.peer(SERVER).coordinator.clone();
    let mobile_id = harness.peer(MOBILE_ANSWERER).id.clone();

    // 1. Verify baseline works
    let baseline = harness
        .peer(MOBILE_ANSWERER)
        .spawn_request(SERVER, "e2e_baseline", 5000);
    let result = tokio::time::timeout(Duration::from_secs(10), baseline)
        .await
        .expect("baseline should not hang")
        .expect("baseline task should not panic");
    assert!(result.is_ok(), "baseline should succeed: {:?}", result);

    // 2. Call begin_network_recovery to set the authoritative recovery status.
    //    This is the same mechanism that fires in production when a network
    //    change triggers ICE restart on the server (offerer).
    let recovered_peers = server_coordinator
        .begin_network_recovery("ice restart started")
        .await;
    assert!(
        recovered_peers.iter().any(|p| p == &mobile_id),
        "mobile should be marked as recovering"
    );

    // 3. Send request from mobile to server while guard is active.
    //    The echo responder will hit the guard when trying to respond,
    //    and with the fix it should retry until the guard clears.
    let request_handle =
        harness
            .peer(MOBILE_ANSWERER)
            .spawn_request(SERVER, "e2e_request_during_guard", 10000);

    // 4. After 500ms, simulate ICE restart completion to clear the guard.
    let mobile_id_clone = mobile_id.clone();
    let server_coordinator_clone = server_coordinator.clone();
    let session_id = get_peer_session_id(&server_coordinator, &mobile_id)
        .await
        .expect("should have session");
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ =
            server_coordinator_clone
                .event_sender()
                .send(ConnectionEvent::IceRestartCompleted {
                    peer_id: mobile_id_clone,
                    session_id,
                    success: true,
                });
    });

    // 5. The request should succeed because echo responder retries after guard clears.
    let result = tokio::time::timeout(Duration::from_secs(15), request_handle)
        .await
        .expect("request should not hang")
        .expect("request task should not panic");

    assert!(
        result.is_ok(),
        "request should succeed after recovery guard clears — \
         response was likely dropped by guard without retry: {:?}",
        result
    );
}
