use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use actr_hyper::lifecycle::{
    CredentialState, DebounceConfig, DefaultNetworkEventProcessor, NetworkEventProcessor,
};
use actr_hyper::transport::{NetworkError, NetworkResult};
use actr_hyper::wire::webrtc::{SignalingClient, SignalingEvent, SignalingStats};
use actr_protocol::{
    AIdCredential, ActrId, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, SignalingEnvelope, UnregisterResponse,
};
use tokio::sync::broadcast;

struct FakeSignalingClient {
    connected: AtomicBool,
    connections: AtomicU64,
    disconnections: AtomicU64,
    event_tx: broadcast::Sender<SignalingEvent>,
}

impl FakeSignalingClient {
    fn new() -> Self {
        let (event_tx, _event_rx) = broadcast::channel(64);
        Self {
            connected: AtomicBool::new(false),
            connections: AtomicU64::new(0),
            disconnections: AtomicU64::new(0),
            event_tx,
        }
    }

    fn stats(&self) -> SignalingStats {
        SignalingStats {
            connections: self.connections.load(Ordering::SeqCst),
            disconnections: self.disconnections.load(Ordering::SeqCst),
            ..SignalingStats::default()
        }
    }
}

#[async_trait::async_trait]
impl SignalingClient for FakeSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        self.connected.store(true, Ordering::SeqCst);
        self.connections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Connected);
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.disconnections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Disconnected {
            reason: actr_hyper::wire::webrtc::DisconnectReason::Manual,
        });
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

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}

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
}

#[tokio::test]
async fn test_network_available_debounced() {
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
    assert_eq!(stats.connections, 2);
    assert_eq!(stats.disconnections, 1);

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
    assert_eq!(stats.connections, 3);
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
    assert_eq!(stats.connections, 2);
    assert_eq!(stats.disconnections, 2);
}

/// Reproduce race condition: Swift sends Network Available and Network Type Changed simultaneously
///
/// Problem flow:
/// 1. T0: Swift sends Network Available event -> Rust processes, records debounce timestamp
/// 2. T0+few ms: Swift sends Network Type Changed event -> Rust starts processing
/// 3. T0+670ms: TypeChanged internally calls process_network_available()
/// 4. Debounce check: 670ms < 2000ms (debounce window), filtered!
/// 5. Result: WebSocket disconnected but no reconnection
///
/// This test verifies this design flaw
#[tokio::test]
async fn test_race_condition_type_changed_internal_call_debounced() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    // Use a longer debounce window (simulating production's 2 seconds)
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(2000),
        },
    ));

    // Simulate Swift sending two events simultaneously
    // Event 1: Network Available (T0)
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

    // Assert: first Available should execute successfully
    // initial connect + connect in process_available = 2
    assert_eq!(
        stats_after_available.connections, 2,
        "First Available should reconnect"
    );
    assert_eq!(
        stats_after_available.disconnections, 1,
        "First Available should disconnect once"
    );
    assert!(client.is_connected(), "Should be connected after Available");

    // Event 2: Network Type Changed (T0+10ms)
    // Simulate Swift sending TypeChanged event almost simultaneously
    tokio::time::sleep(Duration::from_millis(10)).await;
    tracing::info!("📱 [T0+10ms] Swift sends Network Type Changed");

    // Start processing TypeChanged
    // This will:
    // 1. Call process_network_lost() -> disconnect
    // 2. Wait 500ms
    // 3. Call process_network_available() -> filtered by debounce!!!

    processor
        .process_network_type_changed(true, false) // WiFi connected
        .await
        .expect("type changed should not return error");

    // Key check: state after TypeChanged completes
    let stats_after_type_changed = client.get_stats();
    tracing::info!(
        "📊 After TypeChanged: connections={}, disconnections={}",
        stats_after_type_changed.connections,
        stats_after_type_changed.disconnections
    );
    tracing::info!("📊 Is connected: {}", client.is_connected());

    // This demonstrates the BUG!
    // Expected behavior: TypeChanged should reconnect (connected = true)
    // Actual behavior (due to debounce): internal process_network_available is filtered, no reconnection

    // Let's verify this BUG
    let is_connected_after = client.is_connected();
    let final_connections = stats_after_type_changed.connections;
    let final_disconnections = stats_after_type_changed.disconnections;

    // TypeChanged calls:
    // - process_network_lost() -> disconnections + 1
    // - process_network_available() -> but debounced! won't execute connect
    //
    // So:
    // - disconnections should be 2 (Available disconnects once + TypeChanged calls Lost disconnects once)
    // - connections should still be 2 (internal Available debounced, no connect executed)
    // - is_connected should be false!

    tracing::info!("🔍 Verifying race condition:");
    tracing::info!(
        "   - Final connections: {} (expected 2 due to debounce bug)",
        final_connections
    );
    tracing::info!(
        "   - Final disconnections: {} (expected 2)",
        final_disconnections
    );
    tracing::info!(
        "   - Is connected: {} (expected false due to bug)",
        is_connected_after
    );

    // Verify correct behavior: reconnect_internal() should bypass debounce, reconnect successfully
    //
    // TypeChanged internal call chain:
    // 1. process_network_lost() -> disconnections + 1
    // 2. wait 500ms
    // 3. reconnect_internal() -> bypasses debounce, force reconnect -> connections + 1
    //
    // Therefore expected:
    // - connections = 3 (initial + Available + TypeChanged internal reconnect)
    // - disconnections = 2 (Available disconnect + TypeChanged disconnect)
    // - is_connected = true (successful reconnect)

    assert_eq!(
        final_connections, 3,
        "TypeChanged should trigger reconnect via reconnect_internal()"
    );
    assert_eq!(
        final_disconnections, 2,
        "TypeChanged should disconnect once, Available disconnects once"
    );
    assert!(
        is_connected_after,
        "BUG FIX VERIFIED: After TypeChanged, client should be connected because \
         reconnect_internal() bypasses debounce. \
         This proves the fix where internal calls correctly bypass debounce."
    );

    tracing::info!("✅ Debounce bypass working correctly!");
    tracing::info!("   reconnect_internal() successfully bypassed debounce");
    tracing::info!("   TypeChanged completed with successful reconnection");
}

/// Comparison test: TypeChanged should work normally without a prior Available event
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

    // Send TypeChanged directly, without prior Available
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

    // Should work normally in this case
    // TypeChanged will:
    // 1. Lost: disconnect
    // 2. Wait 500ms
    // 3. Available: disconnect + connect
    //
    // But will Available be affected by Lost's debounce internally? Let's see
    // Actually Available and Lost are different event types, they don't share debounce state

    assert!(
        client.is_connected(),
        "Without prior Available event, TypeChanged should complete successfully"
    );
}
