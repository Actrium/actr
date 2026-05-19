use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use actr_protocol::{
    AIdCredential, ActrId, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, SignalingEnvelope, UnregisterResponse,
};
use actr_runtime::lifecycle::{
    CredentialState, DebounceConfig, DefaultNetworkEventProcessor, NetworkEvent,
    NetworkEventHandle, NetworkEventProcessor, NetworkRecoveryAction, process_network_event_batch,
    run_network_event_reconciler, select_network_recovery_action,
};
use actr_runtime::transport::error::{NetworkError, NetworkResult};
use actr_runtime::wire::webrtc::{ConnectionState, SignalingClient, SignalingStats};
use tokio::sync::watch;

struct FakeSignalingClient {
    connected: AtomicBool,
    connections: AtomicU64,
    connect_once_calls: AtomicU64,
    disconnections: AtomicU64,
    state_tx: watch::Sender<ConnectionState>,
    connect_delay: Duration,
    connect_once_delay: Duration,
}

impl FakeSignalingClient {
    fn new() -> Self {
        Self::new_with_delays(Duration::ZERO, Duration::ZERO)
    }

    fn new_with_delays(connect_delay: Duration, connect_once_delay: Duration) -> Self {
        let (state_tx, _state_rx) = watch::channel(ConnectionState::Disconnected);
        Self {
            connected: AtomicBool::new(false),
            connections: AtomicU64::new(0),
            connect_once_calls: AtomicU64::new(0),
            disconnections: AtomicU64::new(0),
            state_tx,
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

    fn publish_connected(&self) {
        self.connected.store(true, Ordering::SeqCst);
        self.connections.fetch_add(1, Ordering::SeqCst);
        let _ = self.state_tx.send(ConnectionState::Connected);
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
        let _ = self.state_tx.send(ConnectionState::Disconnected);
        Ok(())
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

    fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

#[tokio::test]
async fn test_network_available_rebuilds_when_already_connected() {
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
        stats.connections, 2,
        "Available should rebuild an already connected signaling client"
    );
    assert_eq!(
        stats.disconnections, 1,
        "Available should disconnect before rebuilding signaling"
    );

    processor
        .process_network_available()
        .await
        .expect("second available should be debounced");

    let stats = client.get_stats();
    assert_eq!(stats.connections, 2, "debounced call should not reconnect");
    assert_eq!(
        stats.disconnections, 1,
        "debounced call should not disconnect"
    );

    tokio::time::sleep(Duration::from_millis(600)).await;

    processor
        .process_network_available()
        .await
        .expect("available after window should succeed");

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 3,
        "Available after debounce window should rebuild signaling again"
    );
    assert_eq!(stats.disconnections, 2);
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
        stats.connections, 2,
        "Available should rebuild an already connected client"
    );
    assert_eq!(
        stats.disconnections, 2,
        "Lost should disconnect even when Available was processed first"
    );
}

#[tokio::test]
async fn test_direct_available_then_type_changed_rebuilds_each_event_type() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    // 使用较长的防抖窗口（模拟生产环境的 2 秒）
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(2000),
        },
    ));

    tracing::info!("📱 [T0] Swift sends Network Available");
    processor
        .process_network_available()
        .await
        .expect("first available should succeed");

    let stats_after_available = client.get_stats();
    tracing::info!(
        "📊 After Available: connections={}, disconnections={}",
        stats_after_available.connections,
        stats_after_available.disconnections
    );

    assert_eq!(
        stats_after_available.connections, 2,
        "First Available should rebuild connected signaling"
    );
    assert_eq!(
        stats_after_available.disconnections, 1,
        "First Available should disconnect before rebuilding"
    );
    assert!(client.is_connected(), "Should be connected after Available");

    tokio::time::sleep(Duration::from_millis(10)).await;
    tracing::info!("📱 [T0+10ms] Swift sends Network Type Changed");

    processor
        .process_network_type_changed(true, false) // WiFi connected
        .await
        .expect("type changed should not return error");

    let stats_after_type_changed = client.get_stats();
    tracing::info!(
        "📊 After TypeChanged: connections={}, disconnections={}",
        stats_after_type_changed.connections,
        stats_after_type_changed.disconnections
    );
    tracing::info!("📊 Is connected: {}", client.is_connected());

    let is_connected_after = client.is_connected();
    let final_connections = stats_after_type_changed.connections;
    let final_disconnections = stats_after_type_changed.disconnections;

    tracing::info!("🔍 Verifying concise recovery semantics:");
    tracing::info!(
        "   - Final connections: {} (expected 3 after Available + TypeChanged rebuilds)",
        final_connections
    );
    tracing::info!(
        "   - Final disconnections: {} (expected 2)",
        final_disconnections
    );
    tracing::info!("   - Is connected: {} (expected true)", is_connected_after);

    assert_eq!(
        final_connections, 3,
        "TypeChanged should rebuild an already connected signaling client"
    );
    assert_eq!(
        final_disconnections, 2,
        "TypeChanged should disconnect signaling before rebuilding"
    );
    assert!(
        is_connected_after,
        "After TypeChanged, signaling should still be connected"
    );
}

/// 对比测试：当没有预先的 Available 事件时，TypeChanged 应该正常工作
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

    // 直接发送 TypeChanged，没有预先的 Available
    processor
        .process_network_type_changed(true, false)
        .await
        .expect("type changed should succeed");

    let stats = client.get_stats();
    tracing::info!(
        "📊 TypeChanged without prior Available: connections={}, disconnections={}",
        stats.connections,
        stats.disconnections
    );

    assert!(
        client.is_connected(),
        "Without prior Available event, TypeChanged should complete successfully"
    );
    assert_eq!(
        stats.connections, 2,
        "TypeChanged should rebuild connected signaling even when the local state looks healthy"
    );
    assert_eq!(
        stats.disconnections, 1,
        "TypeChanged should disconnect signaling before rebuilding"
    );
}

#[tokio::test]
async fn test_batch_available_type_changed_rebuilds_signaling_once() {
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
        stats.connections, 2,
        "Available + TypeChanged should rebuild an already connected signaling client once"
    );
    assert_eq!(
        stats.disconnections, 1,
        "Available + TypeChanged should disconnect signaling before rebuilding"
    );
    assert_eq!(
        client.connect_once_calls(),
        1,
        "batched restore should perform a single explicit signaling reconnect"
    );
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

#[tokio::test]
async fn test_batch_cleanup_connections_wins_and_preserves_compat_reconnect() {
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

    let results = process_network_event_batch(events, processor).await;

    assert_eq!(results.len(), 3, "each merged request should get a result");
    assert!(results.iter().all(|result| result.success));
    assert!(
        client.is_connected(),
        "cleanup compat should reconnect signaling"
    );

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 2,
        "cleanup compat should preserve exactly one reconnect after the initial connection"
    );
    assert_eq!(
        stats.disconnections, 1,
        "cleanup compat should preserve exactly one signaling disconnect"
    );
}

#[tokio::test]
async fn test_cleanup_available_batch_uses_single_attempt_connect_not_retry_backoff() {
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

    let events = vec![NetworkEvent::CleanupConnections, NetworkEvent::Available];
    assert_eq!(
        select_network_recovery_action(&events),
        NetworkRecoveryAction::CleanupConnectionsCompat
    );

    let results = tokio::time::timeout(
        Duration::from_millis(250),
        process_network_event_batch(events, processor),
    )
    .await
    .expect("network recovery must not be blocked by the regular reconnect backoff path");

    assert_eq!(results.len(), 2, "each merged request should get a result");
    assert!(results.iter().all(|result| result.success));
    assert!(client.is_connected(), "signaling should reconnect");
    assert_eq!(
        client.connect_once_calls(),
        1,
        "network recovery should use the explicit single-attempt connect path"
    );

    let stats = client.get_stats();
    assert_eq!(stats.connections, 2);
    assert_eq!(stats.disconnections, 1);
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
    let (result_tx, result_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new(event_tx, result_rx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();

    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, result_tx, processor, reconciler_shutdown).await;
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
    assert!(client.is_connected());

    let stats = client.get_stats();
    assert_eq!(
        stats.connections, 2,
        "Lost + Available + TypeChanged in one settle window should rebuild signaling once"
    );
    assert_eq!(
        stats.disconnections, 1,
        "Batched restore should disconnect the existing signaling connection once"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn test_repeated_foreground_restore_batches_rebuild_once_per_cycle() {
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
    let (result_tx, result_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new(event_tx, result_rx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();

    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, result_tx, processor, reconciler_shutdown).await;
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
            stats.connections,
            1 + cycle,
            "foreground cycle {} should add exactly one signaling connection",
            cycle
        );
        assert_eq!(
            stats.disconnections, cycle,
            "foreground cycle {} should disconnect exactly once before rebuilding",
            cycle
        );
        assert_eq!(
            client.connect_once_calls(),
            cycle,
            "foreground cycle {} should use exactly one explicit reconnect",
            cycle
        );
    }

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}
