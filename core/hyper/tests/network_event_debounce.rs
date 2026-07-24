// The legacy batch helpers (`select_network_recovery_action`,
// `process_network_event_batch`) are deprecated in favor of the responsive
// reconciler; these compatibility tests still exercise them intentionally.
#![allow(deprecated)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use actr_hyper::lifecycle::{
    AppLifecycleState, CleanupReason, CredentialState, DebounceConfig,
    DefaultNetworkEventProcessor, NetworkAvailability, NetworkEvent, NetworkEventHandle,
    NetworkEventProcessor, NetworkEventRequest, NetworkEventResult, NetworkRecoveryAction,
    NetworkRecoveryError, NetworkSnapshot, NetworkTransportFlags, ReconnectReason,
    process_network_event_batch, run_network_event_reconciler, select_network_recovery_action,
};
use actr_hyper::transport::{NetworkError, NetworkResult};
use actr_hyper::wire::webrtc::{DisconnectReason, SignalingClient, SignalingEvent, SignalingStats};
use actr_protocol::{
    AIdCredential, ActrId, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, SignalingEnvelope, UnregisterResponse,
};
use tokio::sync::broadcast;

struct FakeSignalingClient {
    connected: AtomicBool,
    connections: AtomicU64,
    connect_once_calls: AtomicU64,
    disconnections: AtomicU64,
    probe_calls: AtomicU64,
    probe_success: AtomicBool,
    auto_reconnect_suppressed: AtomicBool,
    suppress_auto_reconnect_calls: AtomicU64,
    resume_auto_reconnect_calls: AtomicU64,
    event_tx: broadcast::Sender<SignalingEvent>,
    connect_delay: Duration,
    connect_once_delay: Duration,
}

impl FakeSignalingClient {
    fn new() -> Self {
        Self::new_with_delays(Duration::ZERO, Duration::ZERO)
    }

    fn new_with_delays(connect_delay: Duration, connect_once_delay: Duration) -> Self {
        let (event_tx, _event_rx) = broadcast::channel(64);
        Self {
            connected: AtomicBool::new(false),
            connections: AtomicU64::new(0),
            connect_once_calls: AtomicU64::new(0),
            disconnections: AtomicU64::new(0),
            probe_calls: AtomicU64::new(0),
            probe_success: AtomicBool::new(true),
            auto_reconnect_suppressed: AtomicBool::new(false),
            suppress_auto_reconnect_calls: AtomicU64::new(0),
            resume_auto_reconnect_calls: AtomicU64::new(0),
            event_tx,
            connect_delay,
            connect_once_delay,
        }
    }

    fn stats(&self) -> SignalingStats {
        SignalingStats {
            connections: self.connections.load(Ordering::SeqCst),
            disconnections: self.disconnections.load(Ordering::SeqCst),
            ..SignalingStats::default()
        }
    }

    fn connect_once_calls(&self) -> u64 {
        self.connect_once_calls.load(Ordering::SeqCst)
    }

    fn probe_calls(&self) -> u64 {
        self.probe_calls.load(Ordering::SeqCst)
    }

    fn set_probe_success(&self, success: bool) {
        self.probe_success.store(success, Ordering::SeqCst);
    }

    fn auto_reconnect_suppressed(&self) -> bool {
        self.auto_reconnect_suppressed.load(Ordering::SeqCst)
    }

    fn suppress_auto_reconnect_calls(&self) -> u64 {
        self.suppress_auto_reconnect_calls.load(Ordering::SeqCst)
    }

    fn resume_auto_reconnect_calls(&self) -> u64 {
        self.resume_auto_reconnect_calls.load(Ordering::SeqCst)
    }

    fn publish_connected(&self) {
        self.connected.store(true, Ordering::SeqCst);
        self.connections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Connected);
    }
}

#[async_trait::async_trait]
impl SignalingClient for FakeSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        if !self.connect_delay.is_zero() {
            tokio::time::sleep(self.connect_delay).await;
        }
        self.publish_connected();
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        self.connect_once_calls.fetch_add(1, Ordering::SeqCst);
        if !self.connect_once_delay.is_zero() {
            tokio::time::sleep(self.connect_once_delay).await;
        }
        self.publish_connected();
        Ok(())
    }

    fn suppress_auto_reconnect(&self) {
        self.suppress_auto_reconnect_calls
            .fetch_add(1, Ordering::SeqCst);
        self.auto_reconnect_suppressed.store(true, Ordering::SeqCst);
    }

    fn resume_auto_reconnect(&self) {
        self.resume_auto_reconnect_calls
            .fetch_add(1, Ordering::SeqCst);
        self.auto_reconnect_suppressed
            .store(false, Ordering::SeqCst);
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.disconnections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Disconnected {
            reason: DisconnectReason::Manual,
        });
        Ok(())
    }

    async fn probe_alive(&self, _timeout: Duration) -> NetworkResult<()> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        if !self.is_connected() {
            return Err(NetworkError::ConnectionError(
                "fake signaling is disconnected".to_string(),
            ));
        }
        if self.probe_success.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(NetworkError::TimeoutError(
                "fake signaling probe timed out".to_string(),
            ))
        }
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::NotImplemented(
            "register request not implemented in fake client".to_string(),
        ))
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        Err(NetworkError::NotImplemented(
            "unregister request not implemented in fake client".to_string(),
        ))
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: actr_protocol::ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Err(NetworkError::NotImplemented(
            "heartbeat not implemented in fake client".to_string(),
        ))
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        Err(NetworkError::NotImplemented(
            "route candidates not implemented in fake client".to_string(),
        ))
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        Err(NetworkError::NotImplemented(
            "get_signing_key not implemented in fake client".to_string(),
        ))
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Err(NetworkError::NotImplemented(
            "send_envelope not implemented in fake client".to_string(),
        ))
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Err(NetworkError::NotImplemented(
            "receive_envelope not implemented in fake client".to_string(),
        ))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn get_stats(&self) -> SignalingStats {
        self.stats()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

/// Wait for the next signaling event matching `pred`, skipping unrelated
/// events instead of assuming the first delivery is the one under test.
///
/// Event acceptance is decoupled from effect completion (RFC-0400 invariant
/// 11), so a caller can no longer infer "the effect ran" from its acceptance
/// reply alone. This observes the effect's actual, authoritative completion
/// off the signaling client's event stream — a real notification, not a
/// sleep — so it also drives a paused clock's timers (offline grace,
/// backoff) forward exactly as far as the effect chain requires.
async fn wait_for_event(
    events: &mut broadcast::Receiver<SignalingEvent>,
    mut pred: impl FnMut(&SignalingEvent) -> bool,
) {
    loop {
        let event = events
            .recv()
            .await
            .expect("signaling event stream should stay open for the duration of the test");
        if pred(&event) {
            return;
        }
    }
}

fn snapshot(
    sequence: u64,
    availability: NetworkAvailability,
    wifi: bool,
    cellular: bool,
    vpn: bool,
) -> NetworkSnapshot {
    snapshot_with_flags(
        sequence,
        availability,
        NetworkTransportFlags {
            wifi,
            cellular,
            ethernet: false,
            vpn,
            other: false,
        },
        false,
        false,
    )
}

fn snapshot_with_flags(
    sequence: u64,
    availability: NetworkAvailability,
    transport: NetworkTransportFlags,
    is_expensive: bool,
    is_constrained: bool,
) -> NetworkSnapshot {
    NetworkSnapshot {
        sequence,
        availability,
        transport,
        is_expensive,
        is_constrained,
    }
}

fn path_event(snapshot: NetworkSnapshot) -> NetworkEvent {
    NetworkEvent::NetworkPathChanged { snapshot }
}

fn online_event(sequence: u64) -> NetworkEvent {
    NetworkEvent::NetworkPathChanged {
        snapshot: snapshot(sequence, NetworkAvailability::Available, true, false, false),
    }
}

fn offline_event(sequence: u64) -> NetworkEvent {
    NetworkEvent::NetworkPathChanged {
        snapshot: snapshot(
            sequence,
            NetworkAvailability::Unavailable,
            false,
            false,
            false,
        ),
    }
}

fn wifi_event(sequence: u64) -> NetworkEvent {
    online_event(sequence)
}

fn cellular_event(sequence: u64) -> NetworkEvent {
    NetworkEvent::NetworkPathChanged {
        snapshot: snapshot(sequence, NetworkAvailability::Available, false, true, false),
    }
}

fn foreground_event(background_duration_ms: u64) -> NetworkEvent {
    NetworkEvent::AppLifecycleChanged {
        state: AppLifecycleState::Foreground {
            background_duration_ms,
        },
    }
}

fn background_event() -> NetworkEvent {
    NetworkEvent::AppLifecycleChanged {
        state: AppLifecycleState::Background,
    }
}

#[test]
fn test_l0_documented_event_action_matrix() {
    struct Case {
        case_id: &'static str,
        events: Vec<NetworkEvent>,
        expected: NetworkRecoveryAction,
    }

    let legacy_available = |sequence| {
        path_event(snapshot(
            sequence,
            NetworkAvailability::Available,
            false,
            false,
            false,
        ))
    };

    let cases = vec![
        Case {
            case_id: "L0-01 empty events",
            events: vec![],
            expected: NetworkRecoveryAction::Noop,
        },
        Case {
            case_id: "L0-02 available",
            events: vec![online_event(1)],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-03 unavailable",
            events: vec![offline_event(1)],
            expected: NetworkRecoveryAction::Offline,
        },
        Case {
            case_id: "L0-04 wifi",
            events: vec![wifi_event(1)],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-05 cellular",
            events: vec![cellular_event(1)],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-06 other transport",
            events: vec![path_event(snapshot_with_flags(
                1,
                NetworkAvailability::Available,
                NetworkTransportFlags {
                    other: true,
                    ..Default::default()
                },
                false,
                false,
            ))],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-07 cleanup",
            events: vec![NetworkEvent::CleanupConnections {
                reason: CleanupReason::ManualReset,
            }],
            expected: NetworkRecoveryAction::CleanupOnly,
        },
        Case {
            case_id: "L0-08 unavailable then available",
            events: vec![offline_event(1), wifi_event(2)],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-09 available then unavailable",
            events: vec![online_event(1), offline_event(2)],
            expected: NetworkRecoveryAction::Offline,
        },
        Case {
            case_id: "L0-10 cleanup before available",
            events: vec![
                NetworkEvent::CleanupConnections {
                    reason: CleanupReason::ManualReset,
                },
                wifi_event(1),
            ],
            expected: NetworkRecoveryAction::CleanupOnly,
        },
        Case {
            case_id: "L0-11 cleanup suppresses later restore",
            events: vec![
                online_event(1),
                NetworkEvent::CleanupConnections {
                    reason: CleanupReason::ManualReset,
                },
                cellular_event(2),
            ],
            expected: NetworkRecoveryAction::CleanupOnly,
        },
        Case {
            case_id: "L0-12 cleanup after available",
            events: vec![
                wifi_event(1),
                NetworkEvent::CleanupConnections {
                    reason: CleanupReason::ManualReset,
                },
            ],
            expected: NetworkRecoveryAction::CleanupOnly,
        },
        Case {
            case_id: "L0-13 background alone noops",
            events: vec![background_event()],
            expected: NetworkRecoveryAction::Noop,
        },
        Case {
            case_id: "L0-14 short foreground probes",
            events: vec![foreground_event(5_000)],
            expected: NetworkRecoveryAction::Probe,
        },
        Case {
            case_id: "L0-15 latest sequence wins after flapping",
            events: vec![
                offline_event(1),
                online_event(2),
                offline_event(3),
                online_event(4),
            ],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-16 legacy available maps to path available",
            events: vec![legacy_available(1)],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-17 latest wifi sequence wins",
            events: vec![offline_event(1), wifi_event(2)],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-18 unknown availability probes",
            events: vec![path_event(snapshot(
                1,
                NetworkAvailability::Unknown,
                false,
                false,
                false,
            ))],
            expected: NetworkRecoveryAction::Probe,
        },
        Case {
            case_id: "L0-19 vpn available restores",
            events: vec![path_event(snapshot_with_flags(
                1,
                NetworkAvailability::Available,
                NetworkTransportFlags {
                    vpn: true,
                    other: true,
                    ..Default::default()
                },
                false,
                false,
            ))],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-20 expensive constrained available stays restore",
            events: vec![path_event(snapshot_with_flags(
                1,
                NetworkAvailability::Available,
                NetworkTransportFlags {
                    cellular: true,
                    ..Default::default()
                },
                true,
                true,
            ))],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-21 long foreground forces reconnect",
            events: vec![foreground_event(60_000)],
            expected: NetworkRecoveryAction::ForceReconnect,
        },
        Case {
            case_id: "L0-22 long foreground and online forces reconnect",
            events: vec![foreground_event(60_000), cellular_event(2)],
            expected: NetworkRecoveryAction::ForceReconnect,
        },
        Case {
            case_id: "L0-23 force reconnect with online path",
            events: vec![
                NetworkEvent::ForceReconnect {
                    reason: ReconnectReason::ManualReconnect,
                },
                online_event(1),
            ],
            expected: NetworkRecoveryAction::ForceReconnect,
        },
        Case {
            case_id: "L0-24 offline suppresses force reconnect",
            events: vec![
                NetworkEvent::ForceReconnect {
                    reason: ReconnectReason::ManualReconnect,
                },
                offline_event(1),
            ],
            expected: NetworkRecoveryAction::Offline,
        },
        Case {
            case_id: "L0-25 app terminating cleanup",
            events: vec![NetworkEvent::CleanupConnections {
                reason: CleanupReason::AppTerminating,
            }],
            expected: NetworkRecoveryAction::CleanupOnly,
        },
        Case {
            case_id: "L0-26 older unavailable snapshot is ignored",
            events: vec![
                path_event(snapshot(
                    2,
                    NetworkAvailability::Available,
                    true,
                    false,
                    true,
                )),
                offline_event(1),
            ],
            expected: NetworkRecoveryAction::Restore,
        },
        Case {
            case_id: "L0-27 long foreground stays offline with offline path",
            events: vec![foreground_event(60_000), offline_event(1)],
            expected: NetworkRecoveryAction::Offline,
        },
        Case {
            case_id: "L0-28 short foreground before online restores",
            events: vec![background_event(), foreground_event(5_000), online_event(1)],
            expected: NetworkRecoveryAction::Restore,
        },
    ];

    for case in cases {
        assert_eq!(
            select_network_recovery_action(&case.events),
            case.expected,
            "{} selected unexpected action for {:?}",
            case.case_id,
            case.events
        );
    }
}

#[tokio::test]
async fn test_l0_duplicate_path_storms_execute_one_settled_action() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    struct Case {
        name: &'static str,
        events: Vec<NetworkEvent>,
        expected_probe_calls: u64,
        expected_connections: u64,
        expected_disconnections: u64,
    }

    let cases = vec![
        Case {
            name: "duplicate_available",
            events: (1..=10).map(online_event).collect(),
            expected_probe_calls: 1,
            expected_connections: 1,
            expected_disconnections: 0,
        },
        Case {
            name: "duplicate_unavailable",
            events: (11..=20).map(offline_event).collect(),
            expected_probe_calls: 1,
            expected_connections: 1,
            expected_disconnections: 1,
        },
    ];

    for case in cases {
        let expected_len = case.events.len();
        let results = process_network_event_batch(case.events, processor.clone()).await;
        assert_eq!(
            results.len(),
            expected_len,
            "{} should return one result per event",
            case.name
        );
        assert!(
            results.iter().all(|result| result.success),
            "{} results should all succeed: {results:?}",
            case.name
        );
        assert_eq!(
            client.probe_calls(),
            case.expected_probe_calls,
            "{} should have expected probe count",
            case.name
        );

        let stats = client.get_stats();
        assert_eq!(
            stats.connections, case.expected_connections,
            "{} should have expected connection count",
            case.name
        );
        assert_eq!(
            stats.disconnections, case.expected_disconnections,
            "{} should have expected disconnection count",
            case.name
        );
    }
}

#[tokio::test]
async fn test_network_available_probes_when_already_connected() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    );

    processor
        .process_network_available()
        .await
        .expect("first available should succeed");

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 1,
        "Available should keep a healthy connected signaling client"
    );
    assert_eq!(
        stats.disconnections, 0,
        "Available should not disconnect when signaling probe succeeds"
    );
    assert_eq!(client.probe_calls(), 1);
    assert_eq!(client.connect_once_calls(), 0);

    processor
        .process_network_available()
        .await
        .expect("second available should be debounced");

    let stats = client.get_stats();
    assert_eq!(stats.connections, 1, "debounced call should not reconnect");
    assert_eq!(
        stats.disconnections, 0,
        "debounced call should not disconnect"
    );
    assert_eq!(client.probe_calls(), 1, "debounced call should not probe");

    tokio::time::sleep(Duration::from_millis(600)).await;

    processor
        .process_network_available()
        .await
        .expect("available after window should succeed");

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 1,
        "Available after debounce window should keep healthy signaling"
    );
    assert_eq!(stats.disconnections, 0);
    assert_eq!(
        client.probe_calls(),
        2,
        "Available after debounce window should probe again"
    );
}

#[tokio::test]
async fn test_supervisor_restore_bypasses_legacy_available_debounce() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_secs(60),
        },
    );

    processor
        .process_network_available()
        .await
        .expect("legacy available call should succeed");
    processor
        .process_network_available()
        .await
        .expect("legacy duplicate should be debounced");
    assert_eq!(client.probe_calls(), 1);

    processor
        .process_network_recovery_action(NetworkRecoveryAction::Restore)
        .await
        .expect("supervisor Restore should execute");
    processor
        .process_network_recovery_action(NetworkRecoveryAction::Restore)
        .await
        .expect("later supervisor Restore should also execute");

    assert_eq!(
        client.probe_calls(),
        3,
        "policy-admitted Restore work must not be swallowed by direct-call debounce"
    );
}

#[tokio::test]
async fn test_network_available_rebuilds_when_signaling_probe_fails() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    client.set_probe_success(false);

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    );

    processor
        .process_network_available()
        .await
        .expect("available should rebuild after failed probe");

    let stats = client.get_stats();
    assert_eq!(client.probe_calls(), 1);
    assert_eq!(
        stats.disconnections, 1,
        "failed probe should disconnect the half-open signaling socket"
    );
    assert_eq!(
        stats.connections, 2,
        "failed probe should reconnect signaling once"
    );
    assert_eq!(client.connect_once_calls(), 1);
    assert!(client.is_connected());
}

#[tokio::test]
async fn test_short_foreground_failed_probe_disconnects_before_restore() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    client.set_probe_success(false);

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let results = process_network_event_batch(vec![foreground_event(5_000)], processor).await;

    assert_eq!(results.len(), 1);
    assert!(results[0].success);
    assert_eq!(
        client.probe_calls(),
        1,
        "failed short-foreground probe should not be probed a second time during restore"
    );
    assert_eq!(client.connect_once_calls(), 1);
    let stats = client.get_stats();
    assert_eq!(
        stats.disconnections, 1,
        "failed short-foreground probe should disconnect stale signaling before restore"
    );
    assert_eq!(stats.connections, 2);
    assert!(client.is_connected());
}

#[tokio::test]
async fn test_short_foreground_reenables_auto_reconnect_without_starting_an_extra_connect() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let reconciler_shutdown = shutdown.clone();
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor_for_task, reconciler_shutdown).await;
    });

    handle
        .handle_app_lifecycle_changed(AppLifecycleState::Background)
        .await
        .expect("background event should complete");
    assert!(client.auto_reconnect_suppressed());
    assert_eq!(client.suppress_auto_reconnect_calls(), 1);

    handle
        .handle_app_lifecycle_changed(AppLifecycleState::Foreground {
            background_duration_ms: 1_000,
        })
        .await
        .expect("short foreground event should complete");

    assert!(!client.auto_reconnect_suppressed());
    assert_eq!(client.resume_auto_reconnect_calls(), 1);
    assert_eq!(
        client.connect_once_calls(),
        0,
        "healthy signaling must not reconnect just to clear suppression"
    );
    assert_eq!(client.probe_calls(), 1);

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
}

#[tokio::test]
async fn test_default_recovery_uses_snapshot_state_instead_of_timed_debounce() {
    assert_eq!(DebounceConfig::default().window, Duration::ZERO);

    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let reconciler_shutdown = shutdown.clone();
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor_for_task, reconciler_shutdown).await;
    });

    for sequence in [1, 2] {
        handle
            .handle_network_path_changed(match online_event(sequence) {
                NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                _ => unreachable!(),
            })
            .await
            .expect("online snapshot should complete");
    }

    assert_eq!(
        client.probe_calls(),
        1,
        "newer equivalent snapshots are suppressed by state, not elapsed time"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
}

#[tokio::test]
async fn test_network_available_connects_without_probe_when_disconnected() {
    let client = Arc::new(FakeSignalingClient::new());

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    );

    processor
        .process_network_available()
        .await
        .expect("available should connect disconnected signaling");

    let stats = client.get_stats();
    assert_eq!(client.probe_calls(), 0);
    assert_eq!(client.connect_once_calls(), 1);
    assert_eq!(stats.connections, 1);
    assert_eq!(stats.disconnections, 0);
    assert!(client.is_connected());
}

#[tokio::test]
async fn test_debounce_does_not_cross_event_types() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    );

    processor
        .process_network_available()
        .await
        .expect("available should succeed");

    processor
        .process_network_lost()
        .await
        .expect("lost should not be debounced by available");

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 1,
        "Available should keep a healthy connected client"
    );
    assert_eq!(
        stats.disconnections, 1,
        "Lost should disconnect even when Available was processed first"
    );
    assert_eq!(client.probe_calls(), 1);
}

#[tokio::test]
async fn test_direct_available_then_type_changed_probes_each_event_type() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(2000),
        },
    ));

    processor
        .process_network_available()
        .await
        .expect("first available should succeed");

    let stats_after_available = client.get_stats();
    assert_eq!(
        stats_after_available.connections, 1,
        "First Available should keep healthy connected signaling"
    );
    assert_eq!(
        stats_after_available.disconnections, 0,
        "First Available should not disconnect healthy signaling"
    );
    assert!(client.is_connected(), "Should be connected after Available");
    assert_eq!(client.probe_calls(), 1);

    tokio::time::sleep(Duration::from_millis(10)).await;

    processor
        .process_network_type_changed(true, false)
        .await
        .expect("type changed should not return error");

    let stats_after_type_changed = client.get_stats();
    assert_eq!(
        stats_after_type_changed.connections, 1,
        "TypeChanged should keep an already healthy signaling client"
    );
    assert_eq!(
        stats_after_type_changed.disconnections, 0,
        "TypeChanged should not disconnect healthy signaling"
    );
    assert_eq!(
        client.probe_calls(),
        2,
        "Available and TypeChanged should each probe when outside their debounce buckets"
    );
    assert!(
        client.is_connected(),
        "After TypeChanged, signaling should still be connected"
    );
}

#[tokio::test]
async fn test_type_changed_works_without_prior_available() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(2000),
        },
    );

    processor
        .process_network_type_changed(true, false)
        .await
        .expect("type changed should succeed");

    let stats = client.get_stats();
    assert!(client.is_connected());
    assert_eq!(
        stats.connections, 1,
        "TypeChanged should keep healthy connected signaling"
    );
    assert_eq!(
        stats.disconnections, 0,
        "TypeChanged should not disconnect signaling when probe succeeds"
    );
    assert_eq!(client.probe_calls(), 1);
    assert_eq!(client.connect_once_calls(), 0);
}

#[tokio::test]
async fn test_batch_available_type_changed_probes_signaling_once() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let action = select_network_recovery_action(&[online_event(1), wifi_event(2)]);
    assert_eq!(action, NetworkRecoveryAction::Restore);

    let results =
        process_network_event_batch(vec![online_event(1), wifi_event(2)], processor).await;

    assert_eq!(results.len(), 2, "each merged request should get a result");
    assert!(results.iter().all(|result| result.success));
    assert!(client.is_connected(), "signaling should remain connected");

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 1,
        "Available + TypeChanged should keep a healthy connected signaling client"
    );
    assert_eq!(
        stats.disconnections, 0,
        "Available + TypeChanged should not disconnect when probe succeeds"
    );
    assert_eq!(
        client.connect_once_calls(),
        0,
        "batched restore should not reconnect when signaling probe succeeds"
    );
    assert_eq!(
        client.probe_calls(),
        1,
        "batched restore should perform one signaling probe"
    );
}

#[tokio::test]
async fn test_batch_restore_rebuilds_once_when_signaling_probe_fails() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    client.set_probe_success(false);

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let results =
        process_network_event_batch(vec![online_event(1), cellular_event(2)], processor).await;

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| result.success));
    assert!(client.is_connected());

    let stats = client.get_stats();
    assert_eq!(client.probe_calls(), 1);
    assert_eq!(
        stats.disconnections, 1,
        "batched restore should disconnect once after failed probe"
    );
    assert_eq!(
        stats.connections, 2,
        "batched restore should reconnect once after failed probe"
    );
    assert_eq!(client.connect_once_calls(), 1);
}

#[tokio::test]
async fn test_batch_lost_available_type_changed_prefers_restore() {
    let client = Arc::new(FakeSignalingClient::new());

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let events = vec![offline_event(1), online_event(2), cellular_event(3)];
    assert_eq!(
        select_network_recovery_action(&events),
        NetworkRecoveryAction::Restore
    );

    let results = process_network_event_batch(events, processor).await;

    assert_eq!(results.len(), 3, "each merged request should get a result");
    assert!(results.iter().all(|result| result.success));
    assert!(
        client.is_connected(),
        "signaling should be connected after restore"
    );

    let stats = client.get_stats();
    assert_eq!(stats.connections, 1);
    assert_eq!(client.connect_once_calls(), 1);
    assert_eq!(
        client.probe_calls(),
        0,
        "disconnected restore should connect directly without probing"
    );
    assert_eq!(
        stats.disconnections, 0,
        "Lost in the same settle batch as restore should not force an extra disconnect"
    );
}

#[test]
fn test_batch_action_uses_latest_network_state_event() {
    let available_last = vec![online_event(1), offline_event(2), online_event(3)];
    assert_eq!(
        select_network_recovery_action(&available_last),
        NetworkRecoveryAction::Restore,
        "Available after Lost means the settled final state is online"
    );

    let lost_last = vec![offline_event(1), online_event(2), offline_event(3)];
    assert_eq!(
        select_network_recovery_action(&lost_last),
        NetworkRecoveryAction::Offline,
        "Lost after Available means the settled final state is offline"
    );
}

// `test_recovery_supervisor_fact_matrix` (pre-RFC-0400) was removed here.
// It drove the deprecated per-instance `RecoverySupervisor::submit_fact` /
// `reconcile` API directly with hand-built `ConnectionFact` values; that API
// is no longer public (the RFC-0400 supervisor rewrite replaced it with the
// translate()-driven engine, and the pre-RFC selector now lives only as the
// crate-private `legacy_select_action` used by `select_network_recovery_action`).
// Every one of its six cases already exercised the identical underlying
// selector through `select_network_recovery_action`'s `NetworkEvent` entry
// point (which converts each event to the same `ConnectionFact` before
// reconciling), so none of it was unique coverage:
//   - "background_only"                    == L0-13
//   - "short_foreground_without_network_fact" == L0-14
//   - "cleanup_suppresses_later_restore"    == L0-11
//   - "latest_snapshot_sequence_wins"       == L0-15 / L0-26
//   - "offline_suppresses_forced_reconnect" == L0-24
//   - "foreground_then_online" had no exact prior case, so it was ported
//     forward as L0-28 in `test_l0_documented_event_action_matrix` above
//     instead of being dropped.

#[tokio::test]
async fn test_cleanup_batches_disconnect_without_reconnect() {
    struct Case {
        name: &'static str,
        events: Vec<NetworkEvent>,
        delayed_connect: bool,
        timeout: Option<Duration>,
    }

    let cases = vec![
        Case {
            name: "cleanup_with_available_and_wifi",
            events: vec![
                NetworkEvent::CleanupConnections {
                    reason: CleanupReason::ManualReset,
                },
                online_event(1),
                wifi_event(2),
            ],
            delayed_connect: false,
            timeout: None,
        },
        Case {
            name: "cleanup_with_available_does_not_enter_reconnect_backoff",
            events: vec![
                NetworkEvent::CleanupConnections {
                    reason: CleanupReason::ManualReset,
                },
                online_event(1),
            ],
            delayed_connect: true,
            timeout: Some(Duration::from_millis(250)),
        },
    ];

    for case in cases {
        let client = if case.delayed_connect {
            let client = Arc::new(FakeSignalingClient::new_with_delays(
                Duration::from_secs(5),
                Duration::ZERO,
            ));
            client.publish_connected();
            client
        } else {
            let client = Arc::new(FakeSignalingClient::new());
            client.connect().await.expect("initial connect");
            client
        };

        let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
            client.clone(),
            None,
            DebounceConfig {
                window: Duration::from_millis(500),
            },
        ));

        assert_eq!(
            select_network_recovery_action(&case.events),
            NetworkRecoveryAction::CleanupOnly,
            "{} should select cleanup only",
            case.name
        );

        let expected_len = case.events.len();
        let results = match case.timeout {
            Some(timeout) => {
                tokio::time::timeout(timeout, process_network_event_batch(case.events, processor))
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "{} must not be blocked by the regular reconnect backoff path",
                            case.name
                        )
                    })
            }
            None => process_network_event_batch(case.events, processor).await,
        };

        assert_eq!(
            results.len(),
            expected_len,
            "{} should return one result per merged request",
            case.name
        );
        assert!(
            results.iter().all(|result| result.success),
            "{} results should all succeed: {results:?}",
            case.name
        );
        assert!(!client.is_connected(), "{} should not reconnect", case.name);
        assert_eq!(
            client.connect_once_calls(),
            0,
            "{} should not connect_once",
            case.name
        );
        assert_eq!(client.probe_calls(), 0, "{} should not probe", case.name);

        let stats = client.get_stats();
        assert_eq!(
            stats.connections, 1,
            "{} initial connection only",
            case.name
        );
        assert_eq!(
            stats.disconnections, 1,
            "{} should preserve exactly one signaling disconnect",
            case.name
        );
    }
}

#[tokio::test(start_paused = true)]
async fn test_network_event_handle_rolls_back_offline_before_grace_expires() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new(event_tx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();

    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });
    let started = tokio::time::Instant::now();

    let lost = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .handle_network_path_changed(match offline_event(1) {
                    NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                    _ => unreachable!(),
                })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    let available = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .handle_network_path_changed(match online_event(2) {
                    NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                    _ => unreachable!(),
                })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    let type_changed = tokio::spawn(async move {
        handle
            .handle_network_path_changed(match wifi_event(3) {
                NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                _ => unreachable!(),
            })
            .await
    });

    let lost_result = lost.await.expect("lost task should not panic").unwrap();
    let available_result = available
        .await
        .expect("available task should not panic")
        .unwrap();
    let type_changed_result = type_changed
        .await
        .expect("type changed task should not panic")
        .unwrap();

    assert!(lost_result.success);
    assert!(available_result.success);
    assert!(type_changed_result.success);
    assert!(matches!(
        lost_result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
    assert!(matches!(
        available_result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
    assert!(matches!(
        type_changed_result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
    assert!(client.is_connected());
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "an available path should roll back pending offline before the grace period expires"
    );

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 1,
        "Fast offline rollback should keep healthy signaling"
    );
    assert_eq!(
        stats.disconnections, 0,
        "Rolled-back offline state should not disconnect when signaling probe succeeds"
    );
    assert_eq!(
        client.probe_calls(),
        1,
        "Equivalent available paths should be structurally deduplicated"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn test_offline_candidate_drained_after_non_offline_head_owns_grace() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let mut events = client.subscribe_events();
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);

    let (online_tx, online_rx) = tokio::sync::oneshot::channel();
    event_tx
        .send(NetworkEventRequest {
            event: online_event(1),
            result_tx: online_tx,
            source_epoch: 1,
            observed_at: tokio::time::Instant::now(),
        })
        .await
        .expect("online event should queue");
    let (offline_tx, offline_rx) = tokio::sync::oneshot::channel();
    event_tx
        .send(NetworkEventRequest {
            event: offline_event(2),
            result_tx: offline_tx,
            source_epoch: 1,
            observed_at: tokio::time::Instant::now(),
        })
        .await
        .expect("offline event should queue behind the online head");

    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let started = tokio::time::Instant::now();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    // Acceptance does not wait for effect completion (invariant 11): both
    // requests are acknowledged as soon as they are dequeued and reconciled,
    // well before the 400ms OfflineCandidate grace timer the offline head
    // arms.
    assert!(
        online_rx
            .await
            .expect("online result sender should remain open")
            .success
    );
    assert!(
        offline_rx
            .await
            .expect("offline result sender should remain open")
            .success
    );

    // The confirmed-offline disconnect only runs once the grace timer
    // expires; observe that completion off the signaling event stream rather
    // than inferring it from the (now-immediate) acceptance reply.
    wait_for_event(&mut events, |event| {
        matches!(event, SignalingEvent::Disconnected { .. })
    })
    .await;

    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "an offline candidate discovered while draining queued requests must still honor its own grace timer"
    );
    assert!(!client.is_connected());
    assert_eq!(client.get_stats().disconnections, 1);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn test_new_offline_candidate_after_fast_rollback_restarts_grace() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let mut events = client.subscribe_events();
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);

    let mut results = Vec::new();
    for event in [offline_event(1), online_event(2), offline_event(3)] {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        event_tx
            .send(NetworkEventRequest {
                event,
                result_tx,
                source_epoch: 1,
                observed_at: tokio::time::Instant::now(),
            })
            .await
            .expect("path event should queue");
        results.push(result_rx);
    }

    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let started = tokio::time::Instant::now();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    // Acceptance does not wait for effect completion (invariant 11): all three
    // requests are acknowledged well before the rolled-back first candidate's
    // grace timer, let alone the second candidate's fresh one, could expire.
    for result in results {
        assert!(
            result
                .await
                .expect("path result sender should remain open")
                .success
        );
    }

    // Only the final (rolled-back-then-recommitted) candidate should ever
    // reach a confirmed disconnect; observe that off the event stream.
    wait_for_event(&mut events, |event| {
        matches!(event, SignalingEvent::Disconnected { .. })
    })
    .await;

    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "a new offline candidate after rollback must start its own grace period"
    );
    assert!(!client.is_connected());
    assert_eq!(
        client.get_stats().disconnections,
        1,
        "only the final committed offline candidate should disconnect"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn test_reconnect_intent_survives_across_reconciler_receive_cycles() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let mut events = client.subscribe_events();

    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    // Acceptance does not wait for effect completion (invariant 11): the
    // offline snapshot is acknowledged immediately, well before its 400ms
    // OfflineCandidate grace timer could expire.
    let offline_result = handle
        .handle_network_path_changed(match offline_event(1) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect("offline event should be accepted");
    assert!(offline_result.success);

    wait_for_event(&mut events, |event| {
        matches!(event, SignalingEvent::Disconnected { .. })
    })
    .await;
    assert!(!client.is_connected());
    assert_eq!(client.get_stats().disconnections, 1);

    let duplicate_offline = handle
        .handle_network_path_changed(match offline_event(2) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect("a later duplicate offline fact should complete");
    assert!(duplicate_offline.success);
    assert_eq!(
        client.get_stats().disconnections,
        1,
        "a committed offline state must not disconnect transports repeatedly"
    );

    let deferred = handle
        .force_reconnect(ReconnectReason::ManualReconnect)
        .await
        .expect("offline reconnect request should be accepted and deferred");
    assert!(deferred.success);
    assert!(!client.is_connected());
    assert_eq!(
        client.connect_once_calls(),
        0,
        "a committed offline path must gate reconnect execution"
    );

    let restored = handle
        .handle_network_path_changed(match online_event(3) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect("available path should be accepted");
    assert!(restored.success);

    // The deferred reconnect intent now executes; observe its completion off
    // the event stream rather than the (already-returned) acceptance reply.
    wait_for_event(&mut events, |event| {
        matches!(event, SignalingEvent::Connected)
    })
    .await;
    assert!(client.is_connected());
    assert_eq!(
        client.connect_once_calls(),
        1,
        "the reconnect intent must survive into a later receive cycle"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

// `test_background_defers_active_recovery_until_foreground` (pre-RFC-0400)
// assumed background always gates active recovery. RFC-0400 scopes that
// gating to the `Gated` lifecycle profile only (see the RFC's "lifecycle
// profile" term and its compatibility section): "the Rust core and headless
// deployments default to `Ungated` and keep recovering exactly as before".
// `run_network_event_reconciler` always constructs `Ungated` (see
// `reconcile_loop`), so under this entry point background never gates a
// reconnect — this is intentional, not a regression. The replacement test
// below verifies the two things that *do* still hold under `Ungated`:
// background alone does not disturb a healthy session, and an explicit
// reconnect is admitted (not silently dropped) while backgrounded. The
// `Gated` profile's actual phase gating cannot be exercised through a public
// reconciler entry point and is covered instead by
// `gated_profile_denies_eligibility_until_foreground` and
// `gated_profile_background_gates_recovery_but_preserves_intent` in
// `recovery_supervisor_tests.rs`.
#[tokio::test]
async fn test_background_preserves_healthy_session_and_admits_reconnect_under_ungated() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let mut events = client.subscribe_events();

    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    assert!(
        handle
            .handle_app_lifecycle_changed(AppLifecycleState::Background)
            .await
            .expect("background fact should complete")
            .success
    );
    assert!(
        client.is_connected(),
        "entering background must not tear down a healthy connection"
    );
    assert_eq!(client.get_stats().disconnections, 0);

    assert!(
        handle
            .handle_network_path_changed(match online_event(1) {
                NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                _ => unreachable!(),
            })
            .await
            .expect("available path should be recorded in background")
            .success
    );
    assert!(
        handle
            .force_reconnect(ReconnectReason::ManualReconnect)
            .await
            .expect("an explicit reconnect must be admitted, not gated, under Ungated")
            .success
    );

    // Acceptance does not wait for effect completion (invariant 11); observe
    // the reconnect's actual completion off the signaling event stream.
    wait_for_event(&mut events, |event| {
        matches!(event, SignalingEvent::Connected)
    })
    .await;
    assert!(client.is_connected());
    assert_eq!(client.connect_once_calls(), 1);
    assert_eq!(client.get_stats().disconnections, 1);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_material_path_updates_bypass_legacy_debounce() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new(event_tx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();

    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    const CYCLES: u64 = 5;

    for cycle in 1..=CYCLES {
        let available = {
            let handle = handle.clone();
            tokio::spawn(async move {
                handle
                    .handle_network_path_changed(match online_event(cycle * 2) {
                        NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                        _ => unreachable!(),
                    })
                    .await
            })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;

        let type_changed = {
            let handle = handle.clone();
            tokio::spawn(async move {
                handle
                    .handle_network_path_changed(match cellular_event(cycle * 2 + 1) {
                        NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                        _ => unreachable!(),
                    })
                    .await
            })
        };

        let available_result = available
            .await
            .expect("available task should not panic")
            .unwrap();
        let type_changed_result = type_changed
            .await
            .expect("type changed task should not panic")
            .unwrap();

        let expected_probes = cycle * 2;
        tokio::time::timeout(Duration::from_secs(1), async {
            while client.probe_calls() < expected_probes {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("policy-admitted probes should complete");

        assert!(
            available_result.success,
            "foreground Available should succeed in cycle {}",
            cycle
        );
        assert!(
            type_changed_result.success,
            "foreground TypeChanged should succeed in cycle {}",
            cycle
        );
        assert!(
            client.is_connected(),
            "signaling should remain connected after foreground cycle {}",
            cycle
        );

        let stats = client.get_stats();
        assert_eq!(
            stats.connections, 1,
            "foreground cycle {} should keep the original healthy signaling connection",
            cycle
        );
        assert_eq!(
            stats.disconnections, 0,
            "foreground cycle {} should not disconnect healthy signaling",
            cycle
        );
        assert_eq!(
            client.connect_once_calls(),
            0,
            "foreground cycle {} should not reconnect healthy signaling",
            cycle
        );
        assert_eq!(
            client.probe_calls(),
            expected_probes,
            "material route changes must not be swallowed by legacy debounce in cycle {}",
            cycle
        );
    }

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_l1_pre_start_queued_event_drains_when_reconciler_starts() {
    let client = Arc::new(FakeSignalingClient::new());
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));

    let pre_start_call = tokio::spawn(async move {
        handle
            .handle_network_path_changed(match online_event(1) {
                NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                _ => unreachable!(),
            })
            .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !pre_start_call.is_finished(),
        "pre-start event should wait while the reconciler is not running"
    );

    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let result = pre_start_call
        .await
        .expect("pre-start event task should not panic")
        .expect("queued pre-start event should complete after reconciler starts");
    assert!(result.success);
    assert!(matches!(
        result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
    assert!(client.is_connected());
    assert_eq!(client.connect_once_calls(), 1);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_l1_old_handle_after_reconciler_shutdown_fails_fast() {
    let client = Arc::new(FakeSignalingClient::new());
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client, None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_millis(100));

    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");

    let err = handle
        .handle_network_path_changed(match online_event(1) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect_err("old handle should fail after reconciler shutdown");

    assert!(
        err.contains("Failed to send network event"),
        "unexpected error: {err}"
    );
}

// `test_l1_reconciler_shutdown_during_offline_grace_is_bounded` (pre-RFC-0400)
// assumed the handle call blocked until the offline grace timer resolved (or
// the caller's own result timeout elapsed), so a shutdown mid-wait had to
// surface as a caller-side timeout error. RFC-0400 invariant 11 decouples
// event acceptance from effect completion: the handle call now returns as
// soon as the snapshot is accepted, well before the 400ms OfflineCandidate
// grace timer could ever fire, so there is no longer a caller left waiting
// during the grace window. The rewritten test instead verifies invariant 30
// directly: shutting the reconciler down while the grace timer (and,
// potentially, its disconnect effect) is still armed terminates it promptly
// rather than hanging on either. No sleep is needed — the acceptance reply
// and `shutdown.cancel()` race nothing, since `select!` is biased towards
// the shutdown branch.
#[tokio::test]
async fn test_l1_reconciler_shutdown_during_offline_grace_is_bounded() {
    let client = Arc::new(FakeSignalingClient::new());
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client, None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_millis(150));

    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let accepted = handle
        .handle_network_path_changed(match offline_event(1) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect("acceptance must not wait for the offline grace timer or its disconnect effect");
    assert!(accepted.success);

    shutdown.cancel();
    reconciler
        .await
        .expect("reconciler task must not panic and must not hang on a pending grace timer");
}

#[tokio::test]
async fn test_l1_command_apis_complete_through_network_event_handle() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let cleanup = handle
        .cleanup_connections(CleanupReason::ManualReset)
        .await
        .expect("cleanup command should complete through handle");
    assert!(cleanup.success);
    assert!(matches!(
        cleanup.event,
        NetworkEvent::CleanupConnections {
            reason: CleanupReason::ManualReset
        }
    ));
    assert!(!client.is_connected());

    let reconnect = handle
        .force_reconnect(ReconnectReason::ManualReconnect)
        .await
        .expect("force reconnect command should complete through handle");
    assert!(reconnect.success);
    assert!(matches!(
        reconnect.event,
        NetworkEvent::ForceReconnect {
            reason: ReconnectReason::ManualReconnect
        }
    ));
    assert!(client.is_connected());
    assert_eq!(client.connect_once_calls(), 1);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_l1_cleanup_is_a_batch_barrier_for_later_recovery_facts() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let (cleanup_tx, cleanup_rx) = tokio::sync::oneshot::channel();
    let (online_tx, online_rx) = tokio::sync::oneshot::channel();

    event_tx
        .try_send(NetworkEventRequest {
            event: NetworkEvent::CleanupConnections {
                reason: CleanupReason::ManualReset,
            },
            result_tx: cleanup_tx,
            source_epoch: 1,
            observed_at: tokio::time::Instant::now(),
        })
        .expect("cleanup should be queued");
    event_tx
        .try_send(NetworkEventRequest {
            event: online_event(1),
            result_tx: online_tx,
            source_epoch: 1,
            observed_at: tokio::time::Instant::now(),
        })
        .expect("online snapshot should be queued after cleanup");

    let shutdown = tokio_util::sync::CancellationToken::new();
    let reconciler_shutdown = shutdown.clone();
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor_for_task, reconciler_shutdown).await;
    });

    assert!(
        cleanup_rx
            .await
            .expect("cleanup result should be delivered")
            .success
    );
    assert!(
        online_rx
            .await
            .expect("online result should be delivered")
            .success
    );
    assert!(
        client.is_connected(),
        "the post-cleanup online fact must execute in its own decision cycle"
    );
    assert_eq!(client.connect_once_calls(), 1);

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
}

#[tokio::test(start_paused = true)]
async fn test_l1_explicit_cleanup_bypasses_offline_grace() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client, None));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let started = tokio::time::Instant::now();
    let cleanup = handle
        .cleanup_connections(CleanupReason::ManualReset)
        .await
        .expect("cleanup command should complete without path settling");

    assert!(cleanup.success);
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "explicit cleanup must not wait for the offline grace period"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_network_event_handle_fails_fast_when_receiver_closed() {
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    drop(event_rx);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_millis(100));

    let err = handle
        .handle_network_path_changed(match online_event(1) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect_err("closed network event receiver should fail");

    assert!(
        err.contains("Failed to send network event"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn test_network_event_handle_pending_request_is_bounded_by_deadline() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(1);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_millis(100));

    let call = tokio::spawn(async move {
        handle
            .handle_network_path_changed(match online_event(1) {
                NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                _ => unreachable!(),
            })
            .await
    });
    let _request = event_rx
        .recv()
        .await
        .expect("request should be queued before timeout");

    let err = call
        .await
        .expect("event call should not panic")
        .expect_err("pending request should time out");

    assert!(
        err.contains("Timed out waiting for network event result"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn test_reconciler_ignores_cancelled_network_event_callers() {
    let client = Arc::new(FakeSignalingClient::new());
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client,
        None,
        DebounceConfig {
            window: Duration::from_millis(10),
        },
    ));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(1));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();

    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let cancelled = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .handle_network_path_changed(match online_event(1) {
                    NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                    _ => unreachable!(),
                })
                .await
        })
    };
    cancelled.abort();

    let result = handle
        .handle_network_path_changed(match offline_event(2) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect("subsequent event should still complete");
    assert!(matches!(
        result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
    assert!(result.success);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_l1_handle_drop_while_event_pending_does_not_poison_reconciler() {
    let client = Arc::new(FakeSignalingClient::new());
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client,
        None,
        DebounceConfig {
            window: Duration::from_millis(10),
        },
    ));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let pending_handle =
        NetworkEventHandle::new_with_result_timeout(event_tx.clone(), Duration::from_secs(1));
    let live_handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));

    let pending_call = tokio::spawn(async move {
        pending_handle
            .handle_network_path_changed(match online_event(1) {
                NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                _ => unreachable!(),
            })
            .await
    });

    tokio::time::sleep(Duration::from_millis(25)).await;
    pending_call.abort();
    let _ = pending_call.await;

    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let result = live_handle
        .handle_network_path_changed(match offline_event(2) {
            NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
            _ => unreachable!(),
        })
        .await
        .expect("new event should complete after old handle was dropped while pending");
    assert!(result.success);
    assert!(matches!(
        result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_network_event_handle_preserves_per_request_result_correlation() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<NetworkEventRequest>(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(1));

    let available = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .handle_network_path_changed(match online_event(1) {
                    NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                    _ => unreachable!(),
                })
                .await
        })
    };
    let lost = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .handle_network_path_changed(match offline_event(2) {
                    NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                    _ => unreachable!(),
                })
                .await
        })
    };

    let first = event_rx.recv().await.expect("first request");
    let second = event_rx.recv().await.expect("second request");

    second
        .result_tx
        .send(NetworkEventResult::success(second.event.clone(), 1))
        .expect("second caller should receive result");
    first
        .result_tx
        .send(NetworkEventResult::success(first.event.clone(), 1))
        .expect("first caller should receive result");

    let available_result = available
        .await
        .expect("available task should not panic")
        .expect("available should complete");
    let lost_result = lost
        .await
        .expect("lost task should not panic")
        .expect("lost should complete");

    assert!(matches!(
        available_result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
    assert!(matches!(
        lost_result.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
}

#[tokio::test]
async fn test_l1_cloned_handles_mixed_concurrent_calls_complete_without_crossed_results() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(50),
        },
    ));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(2));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let network = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .handle_network_path_changed(match online_event(1) {
                    NetworkEvent::NetworkPathChanged { snapshot } => snapshot,
                    _ => unreachable!(),
                })
                .await
        })
    };
    let lifecycle = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .handle_app_lifecycle_changed(AppLifecycleState::Foreground {
                    background_duration_ms: 5_000,
                })
                .await
        })
    };
    let reconnect = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .force_reconnect(ReconnectReason::ManualReconnect)
                .await
        })
    };

    let network = network
        .await
        .expect("network task should not panic")
        .expect("network event should complete");
    let lifecycle = lifecycle
        .await
        .expect("lifecycle task should not panic")
        .expect("lifecycle event should complete");
    let reconnect = reconnect
        .await
        .expect("reconnect task should not panic")
        .expect("reconnect command should complete");

    assert!(matches!(
        network.event,
        NetworkEvent::NetworkPathChanged { .. }
    ));
    assert!(matches!(
        lifecycle.event,
        NetworkEvent::AppLifecycleChanged {
            state: AppLifecycleState::Foreground { .. }
        }
    ));
    assert!(matches!(
        reconnect.event,
        NetworkEvent::ForceReconnect {
            reason: ReconnectReason::ManualReconnect
        }
    ));
    assert!(network.success && lifecycle.success && reconnect.success);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

// ---------------------------------------------------------------------------
// RFC-0400: the supervisor Probe effect reports a typed outcome; the policy —
// not the effect — derives the Restore successor. The deprecated direct-call
// surface keeps the pre-RFC self-healing probe-then-restore contract.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn supervisor_probe_effect_reports_typed_outcome_without_inline_restore() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    client.set_probe_success(false);
    let processor = DefaultNetworkEventProcessor::new(client.clone(), None);

    let err = processor
        .process_network_recovery_effect(NetworkRecoveryAction::Probe)
        .await
        .expect_err("a failed liveness probe must surface its typed outcome");
    assert!(
        matches!(err, NetworkRecoveryError::TransportImpaired { .. }),
        "probe failure must report the conclusive liveness observation: {err:?}"
    );

    assert_eq!(client.probe_calls(), 1);
    let stats = client.get_stats();
    assert_eq!(
        stats.disconnections, 0,
        "the effect must not disconnect: translate() derives the successor"
    );
    assert_eq!(
        client.connect_once_calls(),
        0,
        "the effect must not rebuild inline"
    );
}

#[tokio::test]
async fn legacy_probe_action_retains_self_healing_restore() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    client.set_probe_success(false);
    let processor = DefaultNetworkEventProcessor::new(client.clone(), None);

    processor
        .process_network_recovery_action(NetworkRecoveryAction::Probe)
        .await
        .expect("the direct-call surface keeps probe-then-restore self-healing");

    assert_eq!(client.probe_calls(), 1);
    let stats = client.get_stats();
    assert_eq!(stats.disconnections, 1);
    assert_eq!(client.connect_once_calls(), 1);
    assert!(client.is_connected());
}

#[tokio::test]
async fn route_change_with_live_signaling_probes_then_escalates_to_restore() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let (handle, sink, shutdown, reconciler, mut status_rx) =
        actr_hyper::test_support::spawn_network_event_supervisor_with_status(processor);

    // A live signaling generation committed by the resource owner.
    sink.signaling_generation_committed(1, actr_hyper::lifecycle::SignalingFactOrigin::External);

    // Establish the Online path on Wi-Fi; the derived probe (action 1) runs
    // against the healthy socket, and its completion is reconciled before the
    // route changes.
    assert!(
        handle
            .handle_network_path_changed(snapshot(
                1,
                NetworkAvailability::Available,
                true,
                false,
                false
            ))
            .await
            .expect("initial online snapshot should be accepted")
            .success
    );
    status_rx
        .wait_for(|s| {
            s.last_action_id == Some(1)
                && s.last_outcome == Some(actr_hyper::lifecycle::ObservedOutcome::Succeeded)
        })
        .await
        .expect("the initial healthy probe should settle");
    assert_eq!(client.probe_calls(), 1);

    // The route now migrates Wi-Fi -> cellular and the old socket is half-open:
    // every further liveness probe fails. The supervisor must actively probe
    // (action 2 — not wait 10-15s for an I/O or Pong failure), and translate()
    // must escalate the typed probe failure into a Restore (action 3) that
    // rebuilds the socket.
    client.set_probe_success(false);
    assert!(
        handle
            .handle_network_path_changed(snapshot(
                2,
                NetworkAvailability::Available,
                false,
                true,
                false
            ))
            .await
            .expect("material route change should be accepted")
            .success
    );
    status_rx
        .wait_for(|s| {
            s.last_action_id == Some(3)
                && s.last_outcome == Some(actr_hyper::lifecycle::ObservedOutcome::Succeeded)
        })
        .await
        .expect("the escalated restore should settle");

    assert!(client.is_connected());
    assert_eq!(
        client.probe_calls(),
        3,
        "route probe + the restore's own health probe follow the initial probe"
    );
    assert_eq!(
        client.get_stats().disconnections,
        1,
        "the restore effect (not the probe) disconnects the half-open socket once"
    );
    assert_eq!(client.connect_once_calls(), 1, "exactly one rebuild");

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
}
