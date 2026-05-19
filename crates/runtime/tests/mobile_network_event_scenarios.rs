//! Documented Android/iOS mobile network event scenario coverage.
//!
//! These tests keep the platform-specific tables executable: each documented
//! Android/iOS scenario is mapped to the release SDK events that runtime sees,
//! then processed through the real network event processor with real signaling
//! and WebRTC peers.

mod common;

use std::time::Duration;

use actr_runtime::lifecycle::{
    NetworkEvent, NetworkRecoveryAction, process_network_event_batch,
    select_network_recovery_action,
};
use common::TestHarness;

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
    fn to_event(self) -> NetworkEvent {
        match self {
            EventSpec::Available => NetworkEvent::Available,
            EventSpec::Lost => NetworkEvent::Lost,
            EventSpec::TypeWifi => NetworkEvent::TypeChanged {
                is_wifi: true,
                is_cellular: false,
            },
            EventSpec::TypeCellular => NetworkEvent::TypeChanged {
                is_wifi: false,
                is_cellular: true,
            },
            EventSpec::TypeOther => NetworkEvent::TypeChanged {
                is_wifi: false,
                is_cellular: false,
            },
            EventSpec::CleanupConnections => NetworkEvent::CleanupConnections,
        }
    }
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
        expected_action: NetworkRecoveryAction::CleanupConnectionsCompat,
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
        expected_action: NetworkRecoveryAction::CleanupConnectionsCompat,
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
    specs.iter().map(|spec| spec.to_event()).collect()
}

async fn expect_request_ok(harness: &TestHarness, request_id: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut attempt = 0;
    let mut last_error = String::new();

    loop {
        attempt += 1;
        let attempt_request_id = format!("{}_attempt_{}", request_id, attempt);
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!(
                "{} request did not recover within {:?}; last error: {}",
                request_id, timeout, last_error
            );
        }

        let attempt_timeout = remaining.min(Duration::from_secs(3));
        let handle = harness.peer(100).spawn_request(
            200,
            &attempt_request_id,
            attempt_timeout.as_millis() as u32,
        );

        match tokio::time::timeout(attempt_timeout + Duration::from_millis(250), handle).await {
            Ok(Ok(Ok(response))) => {
                assert!(
                    !response.is_empty(),
                    "{} should receive a non-empty response",
                    request_id
                );
                return;
            }
            Ok(Ok(Err(e))) => {
                last_error = e.to_string();
                if tokio::time::Instant::now() >= deadline {
                    panic!("{} request failed: {}", request_id, last_error);
                }
            }
            Ok(Err(e)) => panic!("{} request task panicked: {}", request_id, e),
            Err(_) => {
                last_error = format!("attempt {} timed out after {:?}", attempt, attempt_timeout);
                if tokio::time::Instant::now() >= deadline {
                    panic!("{} request timed out after {:?}", request_id, timeout);
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn process_scenario(
    platform: &str,
    index: usize,
    scenario: &MobileScenario,
    harness: &TestHarness,
) {
    let label = format!("{}_{}", platform, scenario.name);
    let events = materialize_events(scenario.sdk_events);
    let action = select_network_recovery_action(&events);
    assert_eq!(
        action, scenario.expected_action,
        "{} selected unexpected action for {:?}",
        label, events
    );

    let results =
        process_network_event_batch(events.clone(), harness.peer(100).network_processor()).await;
    assert_eq!(
        results.len(),
        events.len(),
        "{} should return one result per submitted event",
        label
    );
    assert!(
        results.iter().all(|result| result.success),
        "{} should process all events successfully: {:?}",
        label,
        results
    );

    match scenario.expected_action {
        NetworkRecoveryAction::Noop
        | NetworkRecoveryAction::Restore
        | NetworkRecoveryAction::CleanupConnectionsCompat => {
            assert!(
                harness.peer(100).signaling_client.is_connected(),
                "{} should leave signaling connected",
                label
            );
            expect_request_ok(
                harness,
                &format!("{}_verify_{}", platform, index),
                Duration::from_secs(15),
            )
            .await;
        }
        NetworkRecoveryAction::Offline => {
            let restore_events = vec![NetworkEvent::Available];
            let restore_results =
                process_network_event_batch(restore_events, harness.peer(100).network_processor())
                    .await;
            assert!(
                restore_results.iter().all(|result| result.success),
                "{} should be recoverable after offline: {:?}",
                label,
                restore_results
            );
            assert!(
                harness.peer(100).signaling_client.is_connected(),
                "{} should reconnect signaling after follow-up restore",
                label
            );
            expect_request_ok(
                harness,
                &format!("{}_offline_recovered_{}", platform, index),
                Duration::from_secs(15),
            )
            .await;
        }
    }
}

async fn run_documented_scenarios(platform: &str, scenarios: &[MobileScenario]) {
    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;
    harness.connect(100, 200).await;

    for (index, scenario) in scenarios.iter().enumerate() {
        process_scenario(platform, index, scenario, &harness).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_android_documented_network_scenarios() {
    init_tracing();
    run_documented_scenarios("android", ANDROID_SCENARIOS).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_ios_documented_network_scenarios() {
    init_tracing();
    run_documented_scenarios("ios", IOS_SCENARIOS).await;
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

    let recovered_after_outage = vec![
        NetworkEvent::Lost,
        NetworkEvent::Available,
        NetworkEvent::TypeChanged {
            is_wifi: true,
            is_cellular: false,
        },
    ];
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

    let restore_last = vec![
        NetworkEvent::Available,
        NetworkEvent::Lost,
        NetworkEvent::Available,
    ];
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

    let offline_last = vec![
        NetworkEvent::Lost,
        NetworkEvent::Available,
        NetworkEvent::Lost,
    ];
    assert_eq!(
        select_network_recovery_action(&offline_last),
        NetworkRecoveryAction::Offline
    );
    let results =
        process_network_event_batch(offline_last, harness.peer(100).network_processor()).await;
    assert!(results.iter().all(|result| result.success));

    let restore_results = process_network_event_batch(
        vec![NetworkEvent::Available],
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

    let cleanup_then_restore_signals = vec![
        NetworkEvent::CleanupConnections,
        NetworkEvent::Available,
        NetworkEvent::TypeChanged {
            is_wifi: false,
            is_cellular: true,
        },
    ];
    assert_eq!(
        select_network_recovery_action(&cleanup_then_restore_signals),
        NetworkRecoveryAction::CleanupConnectionsCompat
    );
    let results = process_network_event_batch(
        cleanup_then_restore_signals,
        harness.peer(100).network_processor(),
    )
    .await;
    assert!(results.iter().all(|result| result.success));
    expect_request_ok(&harness, "complex_cleanup_rebuild", Duration::from_secs(15)).await;
}
