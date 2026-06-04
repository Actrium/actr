use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use actr_hyper::lifecycle::{
    CleanupReason, CredentialState, DebounceConfig, DefaultNetworkEventProcessor, NetworkEvent,
    NetworkEventHandle, NetworkEventProcessor, NetworkEventRequest, NetworkEventResult,
    NetworkRecoveryAction, NetworkSnapshot, ReconnectReason, process_network_event_batch,
    run_network_event_reconciler, select_network_recovery_action,
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

    async fn send_credential_update_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::NotImplemented(
            "credential update not implemented in fake client".to_string(),
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

    let action = select_network_recovery_action(&[
        NetworkEvent::Available,
        NetworkEvent::TypeChanged {
            is_wifi: true,
            is_cellular: false,
        },
    ]);
    assert_eq!(action, NetworkRecoveryAction::Restore);

    let results = process_network_event_batch(
        vec![
            NetworkEvent::Available,
            NetworkEvent::TypeChanged {
                is_wifi: true,
                is_cellular: false,
            },
        ],
        processor,
    )
    .await;

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

    let results = process_network_event_batch(
        vec![
            NetworkEvent::Available,
            NetworkEvent::TypeChanged {
                is_wifi: false,
                is_cellular: true,
            },
        ],
        processor,
    )
    .await;

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

    let events = vec![
        NetworkEvent::Lost,
        NetworkEvent::Available,
        NetworkEvent::TypeChanged {
            is_wifi: false,
            is_cellular: true,
        },
    ];
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
    let available_last = vec![
        NetworkEvent::Available,
        NetworkEvent::Lost,
        NetworkEvent::Available,
    ];
    assert_eq!(
        select_network_recovery_action(&available_last),
        NetworkRecoveryAction::Restore,
        "Available after Lost means the settled final state is online"
    );

    let lost_last = vec![
        NetworkEvent::Lost,
        NetworkEvent::Available,
        NetworkEvent::Lost,
    ];
    assert_eq!(
        select_network_recovery_action(&lost_last),
        NetworkRecoveryAction::Offline,
        "Lost after Available means the settled final state is offline"
    );
}

#[test]
fn test_snapshot_and_lifecycle_events_select_expected_actions() {
    let offline_snapshot = NetworkSnapshot {
        is_available: false,
        is_wifi: false,
        is_cellular: false,
        is_vpn: false,
        timestamp_ms: 100,
    };
    assert_eq!(
        select_network_recovery_action(&[NetworkEvent::PathChanged {
            snapshot: offline_snapshot,
        }]),
        NetworkRecoveryAction::Offline
    );

    let vpn_snapshot = NetworkSnapshot {
        is_available: true,
        is_wifi: true,
        is_cellular: false,
        is_vpn: true,
        timestamp_ms: 200,
    };
    assert_eq!(
        select_network_recovery_action(&[NetworkEvent::PathChanged {
            snapshot: vpn_snapshot,
        }]),
        NetworkRecoveryAction::Restore
    );

    assert_eq!(
        select_network_recovery_action(&[NetworkEvent::AppEnteredForeground {
            background_duration_ms: 5_000,
            timestamp_ms: 300,
        }]),
        NetworkRecoveryAction::Probe
    );
    assert_eq!(
        select_network_recovery_action(&[NetworkEvent::AppEnteredForeground {
            background_duration_ms: 60_000,
            timestamp_ms: 400,
        }]),
        NetworkRecoveryAction::ForceReconnect
    );
    assert_eq!(
        select_network_recovery_action(&[NetworkEvent::AppTerminating { timestamp_ms: 500 }]),
        NetworkRecoveryAction::CleanupOnly
    );
}

#[test]
fn test_action_priority_cleanup_offline_and_force_reconnect() {
    let cleanup_and_online = vec![
        NetworkEvent::CleanupConnections {
            reason: CleanupReason::ManualReset,
        },
        NetworkEvent::Available,
    ];
    assert_eq!(
        select_network_recovery_action(&cleanup_and_online),
        NetworkRecoveryAction::CleanupOnly,
        "cleanup-only requests must not be turned into reconnects by later online events"
    );

    let force_reconnect = vec![
        NetworkEvent::ForceReconnect {
            reason: ReconnectReason::ManualReconnect,
        },
        NetworkEvent::Available,
    ];
    assert_eq!(
        select_network_recovery_action(&force_reconnect),
        NetworkRecoveryAction::ForceReconnect
    );

    let offline_wins = vec![
        NetworkEvent::ForceReconnect {
            reason: ReconnectReason::ManualReconnect,
        },
        NetworkEvent::Lost,
    ];
    assert_eq!(
        select_network_recovery_action(&offline_wins),
        NetworkRecoveryAction::Offline,
        "offline fact should suppress forced reconnect in the same settled batch"
    );
}

#[tokio::test]
async fn test_batch_cleanup_connections_wins_without_implicit_reconnect() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let events = vec![
        NetworkEvent::CleanupConnections {
            reason: CleanupReason::ManualReset,
        },
        NetworkEvent::Available,
        NetworkEvent::TypeChanged {
            is_wifi: true,
            is_cellular: false,
        },
    ];
    assert_eq!(
        select_network_recovery_action(&events),
        NetworkRecoveryAction::CleanupOnly
    );

    let results = process_network_event_batch(events, processor).await;

    assert_eq!(results.len(), 3, "each merged request should get a result");
    assert!(results.iter().all(|result| result.success));
    assert!(!client.is_connected(), "cleanup only should not reconnect");

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 1,
        "cleanup only should not reconnect after the initial connection"
    );
    assert_eq!(
        stats.disconnections, 1,
        "cleanup only should preserve exactly one signaling disconnect"
    );
    assert_eq!(client.probe_calls(), 0, "cleanup only should not probe");
}

#[tokio::test]
async fn test_cleanup_available_batch_does_not_reconnect() {
    let client = Arc::new(FakeSignalingClient::new_with_delays(
        Duration::from_secs(5),
        Duration::ZERO,
    ));
    client.publish_connected();

    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    ));

    let events = vec![
        NetworkEvent::CleanupConnections {
            reason: CleanupReason::ManualReset,
        },
        NetworkEvent::Available,
    ];
    assert_eq!(
        select_network_recovery_action(&events),
        NetworkRecoveryAction::CleanupOnly
    );

    let results = tokio::time::timeout(
        Duration::from_millis(250),
        process_network_event_batch(events, processor),
    )
    .await
    .expect("network recovery must not be blocked by the regular reconnect backoff path");

    assert_eq!(results.len(), 2, "each merged request should get a result");
    assert!(results.iter().all(|result| result.success));
    assert!(!client.is_connected(), "cleanup only should not reconnect");
    assert_eq!(
        client.connect_once_calls(),
        0,
        "cleanup only should not connect"
    );

    let stats = client.get_stats();
    assert_eq!(stats.connections, 1);
    assert_eq!(stats.disconnections, 1);
    assert_eq!(client.probe_calls(), 0);
}

#[tokio::test]
async fn test_network_event_handle_settle_window_merges_events_once() {
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

    let lost = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.handle_network_lost().await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    let available = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.handle_network_available().await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    let type_changed =
        tokio::spawn(async move { handle.handle_network_type_changed(true, false).await });

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
    assert!(matches!(lost_result.event, NetworkEvent::Lost));
    assert!(matches!(available_result.event, NetworkEvent::Available));
    assert!(matches!(
        type_changed_result.event,
        NetworkEvent::TypeChanged {
            is_wifi: true,
            is_cellular: false
        }
    ));
    assert!(client.is_connected());

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 1,
        "Lost + Available + TypeChanged in one settle window should keep healthy signaling"
    );
    assert_eq!(
        stats.disconnections, 0,
        "Batched restore should not disconnect when signaling probe succeeds"
    );
    assert_eq!(client.probe_calls(), 1, "Batched restore should probe once");

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_repeated_foreground_restore_batches_probe_once_per_cycle() {
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
            tokio::spawn(async move { handle.handle_network_available().await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;

        let type_changed = {
            let handle = handle.clone();
            tokio::spawn(async move {
                handle
                    .handle_network_type_changed(cycle % 2 == 0, cycle % 2 != 0)
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
            cycle,
            "foreground cycle {} should probe once for the settled restore batch",
            cycle
        );
    }

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_network_event_handle_fails_fast_when_receiver_closed() {
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    drop(event_rx);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_millis(100));

    let err = handle
        .handle_network_available()
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

    let call = tokio::spawn(async move { handle.handle_network_available().await });
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
        tokio::spawn(async move { handle.handle_network_available().await })
    };
    cancelled.abort();

    let result = handle
        .handle_network_lost()
        .await
        .expect("subsequent event should still complete");
    assert!(matches!(result.event, NetworkEvent::Lost));
    assert!(result.success);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_network_event_handle_preserves_per_request_result_correlation() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<NetworkEventRequest>(10);
    let handle = NetworkEventHandle::new_with_result_timeout(event_tx, Duration::from_secs(1));

    let available = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.handle_network_available().await })
    };
    let lost = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.handle_network_lost().await })
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

    assert!(matches!(available_result.event, NetworkEvent::Available));
    assert!(matches!(lost_result.event, NetworkEvent::Lost));
}
