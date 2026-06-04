//! Executable coverage for documented Android/iOS network event scenarios.
//!
//! Each documented mobile SDK event sequence is mapped to the events the
//! runtime sees and reconciled into the final recovery action. A separate
//! complex scenario below runs the same reconciled events through the real
//! network event processor with real signaling/WebRTC peers.

use std::time::Duration;

use actr_hyper::lifecycle::{
    CleanupReason, NetworkAvailability, NetworkEvent, NetworkRecoveryAction, NetworkSnapshot,
    NetworkTransportFlags, process_network_event_batch, select_network_recovery_action,
};
use actr_hyper::test_support::TestHarness;

#[derive(Clone, Copy)]
enum EventSpec {
    Available,
    Lost,
    TypeWifi,
    TypeCellular,
    TypeOther,
    CleanupConnections,
}

impl EventSpec {
    fn to_event(self, sequence: u64) -> NetworkEvent {
        match self {
            EventSpec::Available => network_event(sequence, true, false, false),
            EventSpec::Lost => network_event(sequence, false, false, false),
            EventSpec::TypeWifi => network_event(sequence, true, true, false),
            EventSpec::TypeCellular => network_event(sequence, true, false, true),
            EventSpec::TypeOther => network_event(sequence, true, false, false),
            EventSpec::CleanupConnections => NetworkEvent::CleanupConnections {
                reason: CleanupReason::ManualReset,
            },
        }
    }
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

fn online_event(sequence: u64) -> NetworkEvent {
    network_event(sequence, true, false, false)
}

fn offline_event(sequence: u64) -> NetworkEvent {
    network_event(sequence, false, false, false)
}

fn wifi_event(sequence: u64) -> NetworkEvent {
    network_event(sequence, true, true, false)
}

#[derive(Clone, Copy)]
struct MobileScenario {
    name: &'static str,
    sdk_events: &'static [EventSpec],
    expected_action: NetworkRecoveryAction,
}

const A: EventSpec = EventSpec::Available;
const L: EventSpec = EventSpec::Lost;
const TW: EventSpec = EventSpec::TypeWifi;
const TC: EventSpec = EventSpec::TypeCellular;
const TO: EventSpec = EventSpec::TypeOther;
const CC: EventSpec = EventSpec::CleanupConnections;

const ANDROID_SCENARIOS: &[MobileScenario] = &[
    MobileScenario {
        name: "android_cold_start_online",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_cold_start_offline",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "android_wifi_enabled",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_wifi_lost_without_cellular",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "android_wifi_to_cellular_failover",
        sdk_events: &[L, A, TC],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_cellular_to_wifi_with_interleaved_lost",
        sdk_events: &[A, L, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_short_network_flap",
        sdk_events: &[L, A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_airplane_mode_on",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "android_airplane_mode_off",
        sdk_events: &[A, TC],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_vpn_toggle",
        sdk_events: &[TO],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_captive_portal_or_validated_change",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_dns_or_link_properties_change",
        sdk_events: &[TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_metered_change_no_event",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
    },
    MobileScenario {
        name: "android_metered_change_reported_as_type_changed",
        sdk_events: &[TC],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_blocked_status_change",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
    },
    MobileScenario {
        name: "android_background_default_no_cleanup",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
    },
    MobileScenario {
        name: "android_foreground_without_cleanup",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_foreground_legacy_cleanup",
        sdk_events: &[CC, A, TW],
        expected_action: NetworkRecoveryAction::CleanupOnly,
    },
    MobileScenario {
        name: "android_background_network_change_delayed_online",
        sdk_events: &[A, TC],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_background_network_change_delayed_offline",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "android_doze_delayed_callback",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_process_restart_online",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "android_process_restart_offline",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "android_websocket_remote_close_not_a_network_event",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
    },
];

const IOS_SCENARIOS: &[MobileScenario] = &[
    MobileScenario {
        name: "ios_cold_start_online",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_cold_start_offline",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "ios_wifi_to_cellular_with_unsatisfied_gap",
        sdk_events: &[L, A, TC],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_cellular_to_wifi",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_wifi_lost_without_cellular",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "ios_airplane_mode_on",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "ios_airplane_mode_off",
        sdk_events: &[A, TC],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_vpn_or_hotspot_change",
        sdk_events: &[TO],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_low_data_mode_change",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
    },
    MobileScenario {
        name: "ios_expensive_network_change_no_event",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
    },
    MobileScenario {
        name: "ios_expensive_network_change_reported_as_type_changed",
        sdk_events: &[TC],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_route_or_dns_change",
        sdk_events: &[TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_background_default_no_cleanup",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
    },
    MobileScenario {
        name: "ios_foreground_without_cleanup",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_foreground_legacy_cleanup",
        sdk_events: &[CC, A, TW],
        expected_action: NetworkRecoveryAction::CleanupOnly,
    },
    MobileScenario {
        name: "ios_suspended_restore_online",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_suspended_restore_offline",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "ios_multi_scene_duplicate_foreground_events",
        sdk_events: &[A, A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_app_killed_restart_online",
        sdk_events: &[A, TW],
        expected_action: NetworkRecoveryAction::Restore,
    },
    MobileScenario {
        name: "ios_app_killed_restart_offline",
        sdk_events: &[L],
        expected_action: NetworkRecoveryAction::Offline,
    },
    MobileScenario {
        name: "ios_websocket_remote_close_not_a_network_event",
        sdk_events: &[],
        expected_action: NetworkRecoveryAction::Noop,
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

fn materialize_events(specs: &[EventSpec]) -> Vec<NetworkEvent> {
    specs
        .iter()
        .enumerate()
        .map(|(index, spec)| spec.to_event(index as u64 + 1))
        .collect()
}

async fn expect_request_ok(harness: &TestHarness, request_id: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut attempt = 0;

    loop {
        attempt += 1;
        let attempt_id = format!("{request_id}_{attempt}");
        let handle = harness.peer(100).spawn_request(200, &attempt_id, 2_000);

        let last_error = match tokio::time::timeout(Duration::from_secs(3), handle).await {
            Ok(Ok(Ok(response))) => {
                assert!(
                    !response.is_empty(),
                    "{} should receive a non-empty response",
                    request_id
                );
                return;
            }
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
            Ok(Err(err)) => panic!("{} request task panicked: {}", request_id, err),
            Err(_) => format!("{request_id} attempt {attempt} timed out"),
        };

        if tokio::time::Instant::now() >= deadline {
            panic!(
                "{} request failed to recover within {:?}; last error: {}",
                request_id, timeout, last_error
            );
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn assert_scenario_action(platform: &str, scenario: &MobileScenario) {
    let label = format!("{}_{}", platform, scenario.name);
    let events = materialize_events(scenario.sdk_events);
    let action = select_network_recovery_action(&events);
    assert_eq!(
        action, scenario.expected_action,
        "{} selected unexpected action for {:?}",
        label, events
    );
}

fn assert_documented_scenarios(platform: &str, scenarios: &[MobileScenario]) {
    for scenario in scenarios {
        assert_scenario_action(platform, scenario);
    }
}

#[test]
fn test_android_documented_network_scenarios() {
    assert_documented_scenarios("android", ANDROID_SCENARIOS);
}

#[test]
fn test_ios_documented_network_scenarios() {
    assert_documented_scenarios("ios", IOS_SCENARIOS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_complex_mobile_event_storms_with_real_network_outage() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;
    harness.connect(100, 200).await;

    harness.reset_counters();

    harness.simulate_disconnect();
    tokio::time::sleep(Duration::from_secs(8)).await;
    harness.simulate_reconnect();

    let recovered_after_outage = vec![offline_event(1), online_event(2), wifi_event(3)];
    assert_eq!(
        select_network_recovery_action(&recovered_after_outage),
        NetworkRecoveryAction::Restore
    );
    let results = process_network_event_batch(
        recovered_after_outage,
        harness.peer(100).network_processor(),
    )
    .await;
    assert!(results.iter().all(|result| result.success));
    harness
        .wait_for_ice_restart_count(1, Duration::from_secs(10))
        .await;
    expect_request_ok(
        &harness,
        "complex_full_outage_recovered",
        Duration::from_secs(15),
    )
    .await;

    let restore_last = vec![online_event(4), offline_event(5), online_event(6)];
    assert_eq!(
        select_network_recovery_action(&restore_last),
        NetworkRecoveryAction::Restore
    );
    let results =
        process_network_event_batch(restore_last, harness.peer(100).network_processor()).await;
    assert!(results.iter().all(|result| result.success));
    expect_request_ok(
        &harness,
        "complex_available_lost_available",
        Duration::from_secs(15),
    )
    .await;

    let offline_last = vec![offline_event(7), online_event(8), offline_event(9)];
    assert_eq!(
        select_network_recovery_action(&offline_last),
        NetworkRecoveryAction::Offline
    );
    let results =
        process_network_event_batch(offline_last, harness.peer(100).network_processor()).await;
    assert!(results.iter().all(|result| result.success));

    let restore_results = process_network_event_batch(
        vec![online_event(10)],
        harness.peer(100).network_processor(),
    )
    .await;
    assert!(restore_results.iter().all(|result| result.success));
    expect_request_ok(
        &harness,
        "complex_offline_then_restore",
        Duration::from_secs(15),
    )
    .await;
}
