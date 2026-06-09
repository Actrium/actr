//! Mobile full-disconnect recovery tests.
//!
//! The mobile peer is exercised as both WebRTC offerer and answerer. A short
//! semantic recovery window should keep the old WebRTC session and recover via
//! ICE restart. A stale answerer recovery window should close the old session
//! and let the next RPC rebuild it; a stale offerer recovery window should keep
//! using the offerer-driven ICE restart path. Both cases keep the old WebSocket
//! half-open until the restore path probes and rebuilds signaling.

use std::time::{Duration, Instant};

use actr_hyper::lifecycle::{
    NetworkAvailability, NetworkEvent, NetworkRecoveryAction, NetworkSnapshot,
    NetworkTransportFlags, process_network_event_batch, select_network_recovery_action,
};
use actr_hyper::test_support::TestHarness;
use actr_protocol::ActrId;

const ICE_RESTART_SEMANTIC_ELAPSED: Duration = Duration::from_secs(15);
const REBUILD_SEMANTIC_ELAPSED: Duration = Duration::from_secs(65);
const HALF_OPEN_SETTLE: Duration = Duration::from_millis(100);
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
struct RoleCase {
    name: &'static str,
    mobile_serial: u64,
    server_serial: u64,
}

impl RoleCase {
    fn mobile_is_offerer(self) -> bool {
        self.mobile_serial < self.server_serial
    }
}

const ROLE_CASES: [RoleCase; 2] = [
    RoleCase {
        name: "mobile_offerer",
        mobile_serial: 100,
        server_serial: 200,
    },
    RoleCase {
        name: "mobile_answerer",
        mobile_serial: 200,
        server_serial: 100,
    },
];

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_file(true)
        .with_line_number(true)
        .with_test_writer()
        .try_init()
        .ok();
}

fn network_event(sequence: u64, available: bool, wifi: bool, cellular: bool) -> NetworkEvent {
    NetworkEvent::NetworkPathChanged {
        snapshot: NetworkSnapshot {
            sequence,
            availability: if available {
                NetworkAvailability::Available
            } else {
                NetworkAvailability::Unavailable
            },
            transport: NetworkTransportFlags {
                wifi,
                cellular,
                ethernet: false,
                vpn: false,
                other: false,
            },
            is_expensive: false,
            is_constrained: false,
        },
    }
}

async fn setup_mobile_server_harness(case: RoleCase) -> TestHarness {
    let mut harness = TestHarness::with_vnet().await;
    for serial in [
        case.mobile_serial.min(case.server_serial),
        case.mobile_serial.max(case.server_serial),
    ] {
        harness.add_peer(serial).await;
    }
    harness
        .connect(case.mobile_serial, case.server_serial)
        .await;
    harness
}

fn server_id(harness: &TestHarness, case: RoleCase) -> ActrId {
    harness.peer(case.server_serial).id.clone()
}

async fn mobile_peer_session_id(harness: &TestHarness, case: RoleCase) -> Option<u64> {
    harness
        .peer(case.mobile_serial)
        .coordinator
        .peer_session_id_for_test(&server_id(harness, case))
        .await
}

fn assert_event_batch_action(
    label: &str,
    events: &[NetworkEvent],
    expected: NetworkRecoveryAction,
) {
    let action = select_network_recovery_action(events);
    assert_eq!(
        action, expected,
        "{label} should reconcile to {:?}",
        expected
    );
}

async fn process_mobile_events(
    harness: &TestHarness,
    case: RoleCase,
    label: &str,
    events: Vec<NetworkEvent>,
    expected: NetworkRecoveryAction,
) {
    assert_event_batch_action(label, &events, expected);
    let results =
        process_network_event_batch(events, harness.peer(case.mobile_serial).network_processor())
            .await;
    assert!(
        results.iter().all(|result| result.success),
        "{label} network event processing failed: {:?}",
        results
    );
}

async fn expect_mobile_request_ok(
    harness: &TestHarness,
    case: RoleCase,
    request_id: &str,
) -> usize {
    let started = Instant::now();
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let handle =
            harness
                .peer(case.mobile_serial)
                .spawn_request(case.server_serial, request_id, 1000);
        match handle.await {
            Ok(Ok(response)) => {
                tracing::info!(
                    request_id,
                    attempts,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    response_len = response.len(),
                    "mobile request recovered"
                );
                return response.len();
            }
            Ok(Err(err)) => {
                if started.elapsed() >= RECOVERY_TIMEOUT {
                    panic!(
                        "{request_id} did not recover within {:?}: {err}",
                        RECOVERY_TIMEOUT
                    );
                }
                tracing::info!(
                    request_id,
                    attempts,
                    error = %err,
                    "mobile request not ready yet; retrying"
                );
            }
            Err(err) => {
                if started.elapsed() >= RECOVERY_TIMEOUT {
                    panic!("{request_id} task failed and recovery timeout elapsed: {err}",);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_mobile_recovery_cleared(harness: &TestHarness, case: RoleCase, label: &str) {
    let peer_id = server_id(harness, case);
    let deadline = tokio::time::Instant::now() + RECOVERY_TIMEOUT;

    loop {
        if harness
            .peer(case.mobile_serial)
            .coordinator
            .peer_recovery_status(&peer_id)
            .await
            .is_none()
        {
            return;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "{label} did not clear mobile recovery guard within {:?}",
            RECOVERY_TIMEOUT
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn enter_half_open_recovery_window(
    harness: &TestHarness,
    case: RoleCase,
    label: &str,
    semantic_elapsed: Duration,
) {
    let vnet = harness
        .vnet
        .as_ref()
        .expect("half-open recovery test requires VNet");
    let mobile = harness.peer(case.mobile_serial);
    let peer_id = server_id(harness, case);

    vnet.block_network();
    harness.server.pause_forwarding();
    harness.server.blackhole_websocket_io();

    let targets = mobile.coordinator.begin_network_recovery(label).await;
    assert!(
        targets.iter().any(|target| target == &peer_id),
        "{label} should mark the server peer as recovering"
    );
    mobile.coordinator.clear_pending_restarts().await;

    let started_at = Instant::now()
        .checked_sub(semantic_elapsed)
        .expect("semantic elapsed should be representable");
    assert!(
        mobile
            .coordinator
            .force_peer_recovery_started_at_for_test(&peer_id, started_at)
            .await,
        "{label} should be able to age the recovery guard"
    );

    tokio::time::sleep(HALF_OPEN_SETTLE).await;
}

async fn restore_half_open_and_process_network_available(
    harness: &TestHarness,
    case: RoleCase,
    label: &str,
) -> Duration {
    let started = Instant::now();
    let connection_count_before_restore = harness.server.get_connection_count();

    harness.simulate_reconnect();

    process_mobile_events(
        harness,
        case,
        label,
        vec![
            network_event(1, true, false, false),
            network_event(2, true, true, false),
        ],
        NetworkRecoveryAction::Restore,
    )
    .await;

    let connection_count_after_restore = harness.server.get_connection_count();
    assert!(
        connection_count_after_restore > connection_count_before_restore,
        "{label} should rebuild signaling after probing the half-open WebSocket"
    );

    let elapsed = started.elapsed();
    tracing::info!(
        case = label,
        elapsed_ms = elapsed.as_millis() as u64,
        connection_count_before_restore,
        connection_count_after_restore,
        "mobile network restore processed"
    );

    harness.server.restore_websocket_io();
    elapsed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mobile_half_open_15s_semantics_recovers_with_ice_restart() {
    init_tracing();

    for case in ROLE_CASES {
        let label = format!("{}_half_open_15s_ice_restart", case.name);
        let verify_id = format!("{label}_verify");
        let harness = setup_mobile_server_harness(case).await;
        harness.reset_counters();

        let old_session = mobile_peer_session_id(&harness, case)
            .await
            .expect("mobile should have an initial WebRTC session");

        enter_half_open_recovery_window(&harness, case, &label, ICE_RESTART_SEMANTIC_ELAPSED).await;
        restore_half_open_and_process_network_available(&harness, case, &label).await;

        if case.mobile_is_offerer() {
            harness
                .wait_for_ice_restart_count(1, Duration::from_secs(3))
                .await;
        } else {
            harness
                .wait_for_ice_restart_request_count(1, Duration::from_secs(3))
                .await;
        }

        wait_for_mobile_recovery_cleared(&harness, case, &label).await;

        let response_len = expect_mobile_request_ok(&harness, case, &verify_id).await;
        let recovered_session = mobile_peer_session_id(&harness, case)
            .await
            .expect("mobile should still have a WebRTC session after ICE restart");

        assert_eq!(
            recovered_session, old_session,
            "{label} should keep the existing WebRTC session"
        );
        assert!(response_len > 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mobile_half_open_65s_semantics_rebuilds_webrtc() {
    init_tracing();

    for case in ROLE_CASES {
        let label = format!("{}_half_open_65s_rebuild", case.name);
        let verify_id = format!("{label}_verify");
        let harness = setup_mobile_server_harness(case).await;
        harness.reset_counters();

        let old_session = mobile_peer_session_id(&harness, case)
            .await
            .expect("mobile should have an initial WebRTC session");

        enter_half_open_recovery_window(&harness, case, &label, REBUILD_SEMANTIC_ELAPSED).await;
        restore_half_open_and_process_network_available(&harness, case, &label).await;

        if case.mobile_is_offerer() {
            harness
                .wait_for_ice_restart_count(1, Duration::from_secs(3))
                .await;
            wait_for_mobile_recovery_cleared(&harness, case, &label).await;

            let response_len = expect_mobile_request_ok(&harness, case, &verify_id).await;
            let recovered_session = mobile_peer_session_id(&harness, case)
                .await
                .expect("mobile offerer should keep a WebRTC session after ICE restart");

            assert_eq!(
                recovered_session, old_session,
                "{label} should keep the offerer session and recover via ICE restart"
            );
            assert!(response_len > 0);
        } else {
            assert_eq!(
                mobile_peer_session_id(&harness, case).await,
                None,
                "{label} should close the stale answerer session before new sends"
            );
            assert_eq!(
                harness.server.get_ice_restart_request_count(),
                0,
                "{label} should rebuild instead of requesting ICE restart"
            );

            let response_len = expect_mobile_request_ok(&harness, case, &verify_id).await;
            let rebuilt_session = mobile_peer_session_id(&harness, case)
                .await
                .expect("mobile answerer should rebuild a WebRTC session after the next RPC");

            assert_ne!(
                rebuilt_session, old_session,
                "{label} should rebuild with a new WebRTC session"
            );
            assert!(response_len > 0);
        }
    }
}
