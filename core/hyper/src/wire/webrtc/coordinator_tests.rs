use super::*;
use actr_protocol::{
    AIdCredential, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, ServiceAvailabilityState, UnregisterResponse,
};
use tokio::sync::{broadcast, mpsc};

fn test_actor_id(serial_number: u64) -> ActrId {
    ActrId {
        realm: actr_protocol::Realm { realm_id: 1 },
        serial_number,
        r#type: actr_protocol::ActrType {
            manufacturer: "acme".to_string(),
            name: "node".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

fn test_credential() -> AIdCredential {
    AIdCredential {
        key_id: 7,
        claims: Bytes::from_static(b"claims"),
        signature: Bytes::from(vec![0u8; 64]),
    }
}

async fn insert_pending_offer_peer(
    coordinator: &Arc<WebRtcCoordinator>,
    peer_id: ActrId,
    sdp_exchange_id: &str,
) -> u64 {
    let api = webrtc::api::APIBuilder::new().build();
    let peer_connection = Arc::new(
        api.new_peer_connection(Default::default())
            .await
            .expect("test peer connection should be created"),
    );
    let webrtc_conn = WebRtcConnection::new(
        peer_id.clone(),
        peer_connection.clone(),
        coordinator.event_broadcaster.sender(),
    );
    let session_id = webrtc_conn.session_id();
    let (ready_tx, _ready_rx) = oneshot::channel();

    coordinator.peers.write().await.insert(
        peer_id,
        PeerState {
            peer_connection,
            webrtc_conn,
            ready_tx: Some(ready_tx),
            is_offerer: true,
            pending_local_sdp_exchange_id: Some(sdp_exchange_id.to_string()),
            ice_restart_inflight: false,
            ice_restart_attempts: 0,
            restart_task_handle: None,
            restart_wake: Arc::new(tokio::sync::Notify::new()),
            restart_retry_wake: Arc::new(tokio::sync::Notify::new()),
            last_ice_restart_offer_at: None,
            last_state_change: std::time::Instant::now(),
            current_state: RTCPeerConnectionState::New,
            ever_ice_connected: false,
            ever_data_channel_opened: false,
            sendable_hook_reported: false,
            unavailable_hook_reported: false,
            public_hook_state: PublicRtcHookState::Unknown,
            session_id,
            receive_handles: Vec::new(),
        },
    );

    session_id
}

struct CapturingSignalingClient {
    sent: Mutex<Vec<SignalingEnvelope>>,
    event_tx: broadcast::Sender<super::super::SignalingEvent>,
}

impl CapturingSignalingClient {
    fn new() -> Self {
        let (event_tx, _rx) = broadcast::channel(16);
        Self {
            sent: Mutex::new(Vec::new()),
            event_tx,
        }
    }

    async fn last_relay_source(&self) -> ActrId {
        let sent = self.sent.lock().await;
        let envelope = sent.last().expect("relay envelope should be sent");
        let Some(signaling_envelope::Flow::ActrRelay(relay)) = &envelope.flow else {
            panic!("expected ActrRelay envelope");
        };
        relay.source.clone()
    }

    async fn sent_envelopes(&self) -> Vec<SignalingEnvelope> {
        self.sent.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl SignalingClient for CapturingSignalingClient {
    async fn connect(&self) -> crate::transport::NetworkResult<()> {
        Ok(())
    }

    async fn connect_once(&self) -> crate::transport::NetworkResult<()> {
        Ok(())
    }

    async fn disconnect(&self) -> crate::transport::NetworkResult<()> {
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> crate::transport::NetworkResult<RegisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> crate::transport::NetworkResult<UnregisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> crate::transport::NetworkResult<Pong> {
        unimplemented!("not used by this test")
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> crate::transport::NetworkResult<RouteCandidatesResponse> {
        unimplemented!("not used by this test")
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> crate::transport::NetworkResult<(u32, Vec<u8>)> {
        unimplemented!("not used by this test")
    }

    async fn send_envelope(
        &self,
        envelope: SignalingEnvelope,
    ) -> crate::transport::NetworkResult<()> {
        self.sent.lock().await.push(envelope);
        Ok(())
    }

    async fn receive_envelope(&self) -> crate::transport::NetworkResult<Option<SignalingEnvelope>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn get_stats(&self) -> super::super::SignalingStats {
        super::super::SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<super::super::SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

fn new_test_coordinator(local_id: ActrId) -> Arc<WebRtcCoordinator> {
    Arc::new(WebRtcCoordinator::new(
        local_id,
        CredentialState::new(test_credential(), None, None),
        Arc::new(CapturingSignalingClient::new()),
        WebRtcConfig::default(),
        Arc::new(MediaFrameRegistry::new()),
    ))
}

#[tokio::test]
async fn reliable_queue_backpressure_does_not_block_rpc_queue() {
    let coordinator = new_test_coordinator(test_actor_id(1));
    let peer_bytes = test_actor_id(2).encode_to_vec();

    for _ in 0..WEBRTC_RELIABLE_INBOUND_QUEUE_DEPTH {
        coordinator
            .reliable_message_tx
            .try_send((
                peer_bytes.clone(),
                Bytes::from_static(b"reliable"),
                PayloadType::StreamReliable,
            ))
            .expect("reliable queue should accept up to its configured depth");
    }
    assert!(
        coordinator
            .reliable_message_tx
            .try_send((
                peer_bytes.clone(),
                Bytes::from_static(b"overflow"),
                PayloadType::StreamReliable,
            ))
            .is_err(),
        "reliable queue should be full"
    );

    coordinator
        .rpc_message_tx
        .send((
            peer_bytes,
            Bytes::from_static(b"rpc"),
            PayloadType::RpcReliable,
        ))
        .await
        .expect("full reliable queue must not block RPC queue");

    let (_from, data, payload_type) =
        tokio::time::timeout(Duration::from_secs(1), coordinator.receive_rpc_message())
            .await
            .expect("RPC receive should not wait behind reliable backpressure")
            .expect("RPC receive should not fail")
            .expect("RPC message should be present");

    assert_eq!(payload_type, PayloadType::RpcReliable);
    assert_eq!(&data[..], b"rpc");
}

#[tokio::test]
async fn reliable_queue_backpressure_does_not_block_latency_first() {
    let coordinator = new_test_coordinator(test_actor_id(1));
    let peer_bytes = test_actor_id(2).encode_to_vec();

    // Saturate the reliable queue so reliable traffic is backpressured.
    for _ in 0..WEBRTC_RELIABLE_INBOUND_QUEUE_DEPTH {
        coordinator
            .reliable_message_tx
            .try_send((
                peer_bytes.clone(),
                Bytes::from_static(b"reliable"),
                PayloadType::StreamReliable,
            ))
            .expect("reliable queue should accept up to its configured depth");
    }
    assert!(
        coordinator
            .reliable_message_tx
            .try_send((
                peer_bytes.clone(),
                Bytes::from_static(b"overflow"),
                PayloadType::StreamReliable,
            ))
            .is_err(),
        "reliable queue should be full"
    );

    // A full reliable queue must not block latency-first delivery: the two
    // classes have separate queues and receive loops, so a backpressured
    // reliable stream cannot stall latency-first chunks upstream of the
    // registry's drop-newest policy.
    coordinator
        .latency_first_message_tx
        .send((
            peer_bytes,
            Bytes::from_static(b"lf"),
            PayloadType::StreamLatencyFirst,
        ))
        .await
        .expect("full reliable queue must not block latency-first queue");

    let (_from, data, payload_type) = tokio::time::timeout(
        Duration::from_secs(1),
        coordinator.receive_latency_first_message(),
    )
    .await
    .expect("latency-first receive should not wait behind reliable backpressure")
    .expect("latency-first receive should not fail")
    .expect("latency-first message should be present");

    assert_eq!(payload_type, PayloadType::StreamLatencyFirst);
    assert_eq!(&data[..], b"lf");
}

fn install_hook_recorder(
    coordinator: &Arc<WebRtcCoordinator>,
) -> mpsc::UnboundedReceiver<crate::wire::webrtc::HookEvent> {
    let (hook_tx, hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);
    hook_rx
}

async fn expect_disconnected_hook(
    hook_rx: &mut mpsc::UnboundedReceiver<crate::wire::webrtc::HookEvent>,
    peer_id: &ActrId,
    expected_status: WebRtcPeerStatus,
    message: &str,
) {
    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect(message)
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcDisconnected {
            peer_id: got,
            status,
        } => {
            assert_eq!(got, peer_id.clone());
            assert_eq!(status, expected_status);
        }
        other => panic!("unexpected hook event: {other:?}"),
    }
}

#[tokio::test]
async fn webrtc_connected_hook_waits_for_open_data_channel() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.update_connection_state(RTCPeerConnectionState::Connected);
    }

    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);

    coordinator
        .clear_peer_recovering_if_sendable(&peer_id, session_id, "peer connection connected")
        .await;

    let observed = tokio::time::timeout(Duration::from_millis(100), hook_rx.recv()).await;
    assert!(
        observed.is_err(),
        "connected hook must wait for an open DataChannel, got {observed:?}"
    );
}

#[tokio::test]
async fn connecting_state_reopens_connected_hook_window() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.update_connection_state(RTCPeerConnectionState::Connected);
        state.mark_sendable_hook_reported();
    }

    let listener = coordinator.spawn_internal_event_listener();
    coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Connecting,
        });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            {
                let peers = coordinator.peers.read().await;
                let state = peers.get(&peer_id).expect("peer should exist");
                if state.current_state == RTCPeerConnectionState::Connecting
                    && !state.sendable_hook_reported
                {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connecting event should reopen the connected hook window");
    listener.abort();
}

#[tokio::test]
async fn initial_connecting_state_emits_connecting_hook() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);

    let listener = coordinator.spawn_internal_event_listener();
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Connecting,
        });

    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("connecting hook should be emitted")
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcConnectStart { peer_id: got } => {
            assert_eq!(got, peer_id);
        }
        other => panic!("unexpected hook event: {other:?}"),
    }
    listener.abort();
}

#[tokio::test]
async fn initial_failure_emits_idle_not_recovering() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    // Fresh peer that has never reached ICE connected / DataChannel opened,
    // so a failure must terminate at Idle rather than Recovering.
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);

    let listener = coordinator.spawn_internal_event_listener();

    // Initial connecting attempt.
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Connecting,
        });
    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("connecting hook should be emitted")
        .expect("hook channel should remain open");
    assert!(
        matches!(
            event,
            crate::wire::webrtc::HookEvent::WebRtcConnectStart { .. }
        ),
        "unexpected hook event: {event:?}"
    );

    // The attempt fails before the peer ever became usable.
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Disconnected,
        });
    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("failed initial attempt should emit a disconnected hook")
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcDisconnected {
            peer_id: got,
            status,
        } => {
            assert_eq!(got, peer_id);
            assert_eq!(
                status,
                WebRtcPeerStatus::Idle,
                "initial failure must terminate at Idle, not Recovering"
            );
        }
        other => panic!("unexpected hook event: {other:?}"),
    }

    // No Recovering should follow a terminal Idle for a never-connected peer.
    let trailing = tokio::time::timeout(Duration::from_millis(100), hook_rx.recv()).await;
    assert!(
        trailing.is_err(),
        "no further hook expected after terminal Idle, got {trailing:?}"
    );
    listener.abort();
}

#[tokio::test]
async fn recovery_connecting_state_does_not_emit_connecting_hook() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.update_connection_state(RTCPeerConnectionState::Connected);
        state.mark_sendable_hook_reported();
    }

    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);

    let listener = coordinator.spawn_internal_event_listener();
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Disconnected,
        });

    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("disconnected hook should be emitted")
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcDisconnected {
            peer_id: got,
            status,
        } => {
            assert_eq!(got, peer_id);
            assert_eq!(status, WebRtcPeerStatus::Recovering);
        }
        other => panic!("unexpected hook event: {other:?}"),
    }

    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Connecting,
        });
    let connecting = tokio::time::timeout(Duration::from_millis(100), hook_rx.recv()).await;
    assert!(
        connecting.is_err(),
        "recovery Connecting must not emit a public connecting hook, got {connecting:?}"
    );
    listener.abort();
}

#[tokio::test]
async fn data_channel_close_cleanup_emits_terminal_idle_hook() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.update_connection_state(RTCPeerConnectionState::Connected);
        state.mark_data_channel_opened();
        state.mark_sendable_hook_reported();
    }

    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);

    let listener = coordinator.spawn_internal_event_listener();
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::DataChannelClosed {
            peer_id: peer_id.clone(),
            session_id,
            payload_type: PayloadType::RpcReliable,
        });

    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("data channel close should emit recovering hook")
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcDisconnected {
            peer_id: got,
            status,
        } => {
            assert_eq!(got, peer_id);
            assert_eq!(status, WebRtcPeerStatus::Recovering);
        }
        other => panic!("unexpected hook event: {other:?}"),
    }

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !coordinator.peers.read().await.contains_key(&peer_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("data channel close should clean up the peer state");

    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("cleanup should emit terminal idle hook after recovering")
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcDisconnected {
            peer_id: got,
            status,
        } => {
            assert_eq!(got, peer_id);
            assert_eq!(status, WebRtcPeerStatus::Idle);
        }
        other => panic!("unexpected hook event: {other:?}"),
    }

    listener.abort();
}

#[tokio::test]
async fn cancelled_cleanup_emits_terminal_idle_for_connected_peer() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.update_connection_state(RTCPeerConnectionState::Connected);
        state.mark_data_channel_opened();
        state.mark_sendable_hook_reported();
    }

    let mut hook_rx = install_hook_recorder(&coordinator);
    coordinator
        .cleanup_cancelled_connection(&peer_id, "test connected peer replacement")
        .await;

    expect_disconnected_hook(
        &mut hook_rx,
        &peer_id,
        WebRtcPeerStatus::Idle,
        "cancelled cleanup should emit terminal idle for connected peer",
    )
    .await;
    assert!(
        !coordinator.peers.read().await.contains_key(&peer_id),
        "cancelled cleanup should remove peer state"
    );
}

#[tokio::test]
async fn cancelled_cleanup_clears_recovery_guard_for_removed_session() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    coordinator
        .mark_peer_recovering(&peer_id, session_id, "test recovery guard")
        .await;
    assert!(
        coordinator
            .network_recovering_peers
            .read()
            .await
            .contains_key(&peer_id),
        "test setup should mark peer as recovering"
    );

    coordinator
        .cleanup_cancelled_connection(&peer_id, "test recovery guard cleanup")
        .await;

    assert!(
        !coordinator
            .network_recovering_peers
            .read()
            .await
            .contains_key(&peer_id),
        "cancelled cleanup should clear the removed session's recovery guard"
    );
}

#[tokio::test]
async fn cancelled_cleanup_emits_terminal_idle_after_recovering_peer() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.update_connection_state(RTCPeerConnectionState::Connected);
        state.mark_data_channel_opened();
        state.mark_sendable_hook_reported();
    }

    let mut hook_rx = install_hook_recorder(&coordinator);
    let listener = coordinator.spawn_internal_event_listener();
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Failed,
        });

    expect_disconnected_hook(
        &mut hook_rx,
        &peer_id,
        WebRtcPeerStatus::Recovering,
        "failed state should emit recovering hook before stale cleanup",
    )
    .await;

    coordinator
        .cleanup_cancelled_connection(&peer_id, "test stale failed peer cleanup")
        .await;
    expect_disconnected_hook(
        &mut hook_rx,
        &peer_id,
        WebRtcPeerStatus::Idle,
        "cancelled cleanup should emit terminal idle after recovering",
    )
    .await;
    assert!(
        !coordinator.peers.read().await.contains_key(&peer_id),
        "cancelled cleanup should remove peer state"
    );
    listener.abort();
}

#[tokio::test]
async fn failed_ice_restart_after_recovering_emits_terminal_idle() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.update_connection_state(RTCPeerConnectionState::Connected);
        state.mark_data_channel_opened();
        state.mark_sendable_hook_reported();
    }

    let mut hook_rx = install_hook_recorder(&coordinator);
    let listener = coordinator.spawn_internal_event_listener();
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Disconnected,
        });

    expect_disconnected_hook(
        &mut hook_rx,
        &peer_id,
        WebRtcPeerStatus::Recovering,
        "disconnected state should emit recovering hook",
    )
    .await;

    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::IceRestartCompleted {
            peer_id: peer_id.clone(),
            session_id,
            success: false,
        });
    expect_disconnected_hook(
        &mut hook_rx,
        &peer_id,
        WebRtcPeerStatus::Idle,
        "failed ICE restart should emit terminal idle after recovering",
    )
    .await;
    listener.abort();
}

#[tokio::test]
async fn failed_ice_restart_after_public_connecting_emits_disconnected() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);

    let listener = coordinator.spawn_internal_event_listener();
    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id,
            state: ConnectionState::Connecting,
        });
    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("connecting hook should be emitted")
        .expect("hook channel should remain open");
    assert!(
        matches!(
            event,
            crate::wire::webrtc::HookEvent::WebRtcConnectStart { .. }
        ),
        "unexpected hook event: {event:?}"
    );

    let _ = coordinator
        .event_broadcaster
        .send(ConnectionEvent::IceRestartCompleted {
            peer_id: peer_id.clone(),
            session_id,
            success: false,
        });
    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("failed restart should emit disconnected hook")
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcDisconnected {
            peer_id: got,
            status,
        } => {
            assert_eq!(got, peer_id);
            assert_eq!(status, WebRtcPeerStatus::Idle);
        }
        other => panic!("unexpected hook event: {other:?}"),
    }
    listener.abort();
}

#[tokio::test]
async fn webrtc_disconnected_hook_is_session_guarded_and_deduped() {
    let local_id = test_actor_id(1);
    let peer_id = test_actor_id(99);
    let coordinator = new_test_coordinator(local_id);
    let session_id =
        insert_pending_offer_peer(&coordinator, peer_id.clone(), "current-exchange").await;

    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel();
    let hook: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_tx = hook_tx.clone();
        Box::pin(async move {
            let _ = hook_tx.send(event);
        })
    });
    coordinator.set_hook_callback(hook);

    coordinator
        .notify_webrtc_recovering_once(&peer_id, session_id + 1, "stale session")
        .await;
    let stale = tokio::time::timeout(Duration::from_millis(100), hook_rx.recv()).await;
    assert!(
        stale.is_err(),
        "stale session must not emit disconnected hook, got {stale:?}"
    );

    coordinator
        .notify_webrtc_recovering_once(&peer_id, session_id, "peer state Disconnected")
        .await;
    let event = tokio::time::timeout(Duration::from_secs(1), hook_rx.recv())
        .await
        .expect("disconnected hook should be emitted")
        .expect("hook channel should remain open");
    match event {
        crate::wire::webrtc::HookEvent::WebRtcDisconnected {
            peer_id: got,
            status,
        } => {
            assert_eq!(got, peer_id);
            assert_eq!(status, WebRtcPeerStatus::Recovering);
        }
        other => panic!("unexpected hook event: {other:?}"),
    }

    coordinator
        .notify_webrtc_recovering_once(&peer_id, session_id, "duplicate unavailable event")
        .await;
    let duplicate = tokio::time::timeout(Duration::from_millis(100), hook_rx.recv()).await;
    assert!(
        duplicate.is_err(),
        "duplicate unavailable event must not emit another hook, got {duplicate:?}"
    );
}

#[tokio::test]
async fn relay_source_uses_updated_local_id_after_re_registration() {
    let initial_id = test_actor_id(1);
    let renewed_id = test_actor_id(2);
    let target_id = test_actor_id(99);
    let credential_state = CredentialState::new(test_credential(), None, None);
    let signaling_client = Arc::new(CapturingSignalingClient::new());
    let coordinator = WebRtcCoordinator::new(
        initial_id,
        credential_state,
        signaling_client.clone(),
        WebRtcConfig::default(),
        Arc::new(MediaFrameRegistry::new()),
    );

    coordinator.set_local_id(renewed_id.clone()).await;
    coordinator
        .send_actr_relay(
            &target_id,
            actr_relay::Payload::IceCandidate(actr_protocol::IceCandidate {
                candidate: "candidate:0 1 UDP 1 127.0.0.1 9 typ host".to_string(),
                sdp_mid: None,
                sdp_mline_index: None,
                username_fragment: None,
            }),
        )
        .await
        .expect("relay should be sent");

    assert_eq!(signaling_client.last_relay_source().await, renewed_id);
}

#[tokio::test]
async fn actr_relay_answer_can_carry_sdp_exchange_id() {
    let local_id = test_actor_id(1);
    let target_id = test_actor_id(99);
    let credential_state = CredentialState::new(test_credential(), None, None);
    let signaling_client = Arc::new(CapturingSignalingClient::new());
    let coordinator = WebRtcCoordinator::new(
        local_id,
        credential_state,
        signaling_client.clone(),
        WebRtcConfig::default(),
        Arc::new(MediaFrameRegistry::new()),
    );

    let payload = actr_relay::Payload::SessionDescription(actr_protocol::SessionDescription {
        r#type: SdpType::Answer as i32,
        sdp: "answer-sdp".to_string(),
        sdp_exchange_id: Some("exchange-1".to_string()),
    });
    coordinator
        .send_actr_relay(&target_id, payload)
        .await
        .expect("relay answer should be sent");

    let sent = signaling_client.sent_envelopes().await;
    assert_eq!(sent.len(), 1);
    let envelope = &sent[0];
    assert!(envelope.reply_for.is_none());
    let Some(signaling_envelope::Flow::ActrRelay(relay)) = &envelope.flow else {
        panic!("expected ActrRelay envelope");
    };
    let Some(actr_relay::Payload::SessionDescription(sd)) = relay.payload.as_ref() else {
        panic!("expected SessionDescription payload");
    };
    assert_eq!(sd.r#type(), SdpType::Answer);
    assert_eq!(sd.sdp, "answer-sdp");
    assert_eq!(sd.sdp_exchange_id.as_deref(), Some("exchange-1"));
}

#[tokio::test]
async fn stale_answer_sdp_exchange_id_does_not_consume_pending_offer() {
    let local_id = test_actor_id(1);
    let target_id = test_actor_id(99);
    let credential_state = CredentialState::new(test_credential(), None, None);
    let signaling_client = Arc::new(CapturingSignalingClient::new());
    let coordinator = Arc::new(WebRtcCoordinator::new(
        local_id,
        credential_state,
        signaling_client,
        WebRtcConfig::default(),
        Arc::new(MediaFrameRegistry::new()),
    ));

    insert_pending_offer_peer(&coordinator, target_id.clone(), "current-exchange").await;

    coordinator
        .handle_answer(
            &target_id,
            "stale-answer-sdp".to_string(),
            Some("old-exchange".to_string()),
        )
        .await
        .expect("stale answer should be ignored without error");

    let peers = coordinator.peers.read().await;
    let state = peers.get(&target_id).expect("peer should remain");
    assert!(
        state.ready_tx.is_some(),
        "stale Answer must not consume the initial connection ready signal"
    );
    let pending = state
        .pending_local_sdp_exchange_id
        .as_deref()
        .expect("stale Answer must not clear the active pending offer");
    assert_eq!(pending, "current-exchange");
}

#[tokio::test]
async fn clear_pending_restarts_clears_pending_sdp_exchange() {
    let local_id = test_actor_id(1);
    let target_id = test_actor_id(99);
    let credential_state = CredentialState::new(test_credential(), None, None);
    let signaling_client = Arc::new(CapturingSignalingClient::new());
    let coordinator = Arc::new(WebRtcCoordinator::new(
        local_id,
        credential_state,
        signaling_client,
        WebRtcConfig::default(),
        Arc::new(MediaFrameRegistry::new()),
    ));

    insert_pending_offer_peer(&coordinator, target_id.clone(), "restart-exchange").await;
    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&target_id).expect("peer should exist");
        state.ice_restart_inflight = true;
        state.ice_restart_attempts = 1;
    }

    coordinator.clear_pending_restarts().await;

    let peers = coordinator.peers.read().await;
    let state = peers.get(&target_id).expect("peer should remain");
    assert!(
        state.pending_local_sdp_exchange_id.is_none(),
        "aborted ICE restart must not leave a stale pending SDP exchange"
    );
    assert!(!state.ice_restart_inflight);
    assert_eq!(state.ice_restart_attempts, 0);
}

#[test]
fn test_exponential_backoff_basic() {
    // Test basic exponential backoff: 5s -> 10s (capped)
    let mut backoff = ExponentialBackoff::new(
        Duration::from_secs(5),  // initial
        Duration::from_secs(10), // max
        Some(5),                 // max retries
    );

    // First delay: 5s
    assert_eq!(backoff.next(), Some(Duration::from_secs(5)));
    // Second delay: 10s (5*2 = 10, at max)
    assert_eq!(backoff.next(), Some(Duration::from_secs(10)));
    // Third delay: 10s (capped at max)
    assert_eq!(backoff.next(), Some(Duration::from_secs(10)));
    // Fourth delay: 10s
    assert_eq!(backoff.next(), Some(Duration::from_secs(10)));
    // Fifth delay: 10s
    assert_eq!(backoff.next(), Some(Duration::from_secs(10)));
    // Sixth: None (max retries reached)
    assert_eq!(backoff.next(), None);
}

#[test]
fn test_exponential_backoff_sequence_1_2_4_5() {
    // Test the exact ICE restart sequence: 1s -> 2s -> 4s -> 5s...
    let mut backoff = ExponentialBackoff::new(
        Duration::from_millis(ICE_RESTART_INITIAL_BACKOFF_MS),
        Duration::from_millis(ICE_RESTART_MAX_BACKOFF_MS),
        Some(10),
    );

    let delays: Vec<Duration> = backoff.by_ref().take(6).collect();

    assert_eq!(delays[0], Duration::from_secs(1)); // 1s
    assert_eq!(delays[1], Duration::from_secs(2)); // 2s
    assert_eq!(delays[2], Duration::from_secs(4)); // 4s
    assert_eq!(delays[3], Duration::from_secs(5)); // 5s (capped)
    assert_eq!(delays[4], Duration::from_secs(5)); // 5s
    assert_eq!(delays[5], Duration::from_secs(5)); // 5s
}

#[test]
fn test_exponential_backoff_with_total_duration() {
    // Test that with_total_duration sets up the backoff correctly
    // Verify behavior: backoff should produce delays until total duration is exceeded
    let mut backoff = ExponentialBackoff::with_total_duration(
        Duration::from_millis(100), // initial
        Duration::from_millis(200), // max
        Some(5),                    // max retries
        Duration::from_secs(60),    // total duration
    );

    // Should produce at least one delay since total duration is large
    let first = backoff.next();
    assert!(first.is_some(), "should produce at least one delay");
    assert_eq!(first.unwrap(), Duration::from_millis(100));
}

#[test]
fn test_exponential_backoff_no_max_retries() {
    // Test backoff without retry limit (None)
    let mut backoff = ExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(4),
        None, // no retry limit
    );

    // Should continue indefinitely (we just test a few)
    assert_eq!(backoff.next(), Some(Duration::from_secs(1)));
    assert_eq!(backoff.next(), Some(Duration::from_secs(2)));
    assert_eq!(backoff.next(), Some(Duration::from_secs(4))); // capped
    assert_eq!(backoff.next(), Some(Duration::from_secs(4)));
    assert_eq!(backoff.next(), Some(Duration::from_secs(4)));
    // Would continue forever...
}

#[test]
fn test_exponential_backoff_max_delay_cap() {
    // Test that delay is properly capped at max AFTER initial
    // Note: The first call returns initial_delay, then it's doubled and capped
    let mut backoff = ExponentialBackoff::new(
        Duration::from_secs(8),  // initial
        Duration::from_secs(10), // max
        Some(4),
    );

    // First: 8s (initial, not capped yet)
    assert_eq!(backoff.next(), Some(Duration::from_secs(8)));
    // Second: 10s (8*2=16, capped to 10)
    assert_eq!(backoff.next(), Some(Duration::from_secs(10)));
    // Third: 10s (capped)
    assert_eq!(backoff.next(), Some(Duration::from_secs(10)));
    // Fourth: 10s
    assert_eq!(backoff.next(), Some(Duration::from_secs(10)));
    // Fifth: None (max retries reached)
    assert_eq!(backoff.next(), None);
}

#[test]
fn codec_to_payload_type_maps_known_and_unknown() {
    assert_eq!(WebRtcCoordinator::codec_to_payload_type("VP8"), 96);
    assert_eq!(WebRtcCoordinator::codec_to_payload_type("H264"), 97);
    assert_eq!(WebRtcCoordinator::codec_to_payload_type("VP9"), 98);
    assert_eq!(WebRtcCoordinator::codec_to_payload_type("OPUS"), 111);
    // Case-insensitive.
    assert_eq!(WebRtcCoordinator::codec_to_payload_type("h264"), 97);
    assert_eq!(WebRtcCoordinator::codec_to_payload_type("opus"), 111);
    // Unknown codec falls back to 96 (VP8 default).
    assert_eq!(WebRtcCoordinator::codec_to_payload_type("AV1"), 96);
    assert_eq!(WebRtcCoordinator::codec_to_payload_type(""), 96);
}

#[test]
fn stale_peer_reap_uses_dedicated_disconnected_threshold() {
    use RTCPeerConnectionState::*;

    // Terminal states (Failed/Closed) are reaped after MAX_FAILED_DURATION (60s).
    assert!(stale_peer_reap_reason(Failed, Duration::from_secs(61)).is_some());
    assert!(stale_peer_reap_reason(Closed, Duration::from_secs(61)).is_some());
    assert!(stale_peer_reap_reason(Failed, Duration::from_secs(59)).is_none());
    assert!(stale_peer_reap_reason(Closed, Duration::from_secs(59)).is_none());

    // Disconnected gets the dedicated 90s threshold (restart budget + margin):
    // reaped past 90s ...
    assert!(stale_peer_reap_reason(Disconnected, Duration::from_secs(91)).is_some());
    // ... but NOT at the terminal-state threshold, where an ICE restart
    // (60s budget, measured from its trigger, not from the state change)
    // may still legitimately be recovering the peer ...
    assert!(stale_peer_reap_reason(Disconnected, Duration::from_secs(61)).is_none());
    assert!(stale_peer_reap_reason(Disconnected, Duration::from_secs(90)).is_none());
    // ... and a fresh Disconnected peer is never touched.
    assert!(stale_peer_reap_reason(Disconnected, Duration::from_secs(5)).is_none());

    // Healthy/transient states are never reaped, no matter how old.
    for state in [New, Connecting, Connected] {
        assert!(
            stale_peer_reap_reason(state, Duration::from_secs(3600)).is_none(),
            "{state:?} must never be reaped"
        );
    }
}

/// The health check reads the REAL-TIME connection state from the
/// RTCPeerConnection, not the cached `current_state`, so a peer whose pc is
/// healthy must survive even with an ancient `last_state_change`.
#[tokio::test]
async fn health_check_keeps_healthy_peer_with_stale_timestamp() {
    let coordinator = new_test_coordinator(test_actor_id(1));
    let peer_id = test_actor_id(2);
    insert_pending_offer_peer(&coordinator, peer_id.clone(), "exchange-hc").await;

    // Backdate the state-change timestamp far past every reap threshold.
    // (checked_sub keeps the test valid on hosts with a short Instant epoch;
    // the fallback leaves the timestamp fresh, which still must not reap.)
    {
        let mut peers = coordinator.peers.write().await;
        let state = peers.get_mut(&peer_id).expect("peer should exist");
        state.last_state_change = std::time::Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(std::time::Instant::now);
    }

    coordinator.check_and_cleanup_stale_connections().await;

    assert!(
        coordinator.peers.read().await.contains_key(&peer_id),
        "a peer whose RTCPeerConnection reports a healthy state (New) must \
         not be reaped regardless of last_state_change age"
    );
}

#[test]
fn is_ipv4_candidate_allowed_filters_ipv6_and_accepts_ipv4() {
    // IPv6 candidates are rejected.
    assert!(!is_ipv4_candidate_allowed("candidate:... fe80::1 ..."));
    assert!(!is_ipv4_candidate_allowed("candidate:... udp6 ..."));
    assert!(!is_ipv4_candidate_allowed("candidate:... ::1 ..."));

    // IPv4 candidates are accepted (loopback, private, public).
    assert!(is_ipv4_candidate_allowed("candidate:... 127.0.0.1 ..."));
    assert!(is_ipv4_candidate_allowed("candidate:... 192.168.1.10 ..."));
    assert!(is_ipv4_candidate_allowed("candidate:... 10.0.0.5 ..."));
    // No IPv6 marker → accepted.
    assert!(is_ipv4_candidate_allowed(
        "candidate:... 203.0.113.7 udp ..."
    ));
}
