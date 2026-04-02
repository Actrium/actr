//! Mock signaling server for Actor-RTC integration tests.
//!
//! Provides a fully functional WebSocket signaling server that handles
//! registration, heartbeat, route discovery, and relay forwarding without
//! requiring a real actrix instance.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrRelay, ActrType, RegisterResponse, RouteCandidatesResponse,
    SignalingEnvelope, SignalingToActr, TurnCredential, actr_relay, actr_to_signaling,
    peer_to_signaling, register_response, route_candidates_response, signaling_envelope,
    signaling_to_actr,
};
use ed25519_dalek::SigningKey;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// A registered actor entry in the mock server's registry.
#[derive(Clone, Debug)]
struct RegisteredActor {
    actr_id: ActrId,
    actr_type: ActrType,
    /// WebSocket client sender for this actor.
    client_id: String,
    /// WebSocket direct-connect address (if registered).
    ws_address: Option<String>,
    /// Service specification (if provided during registration).
    service_spec: Option<actr_protocol::ServiceSpec>,
}

/// Mock signaling server for integration tests.
///
/// Handles the full signaling lifecycle:
/// - `PeerToSignaling(RegisterRequest)` → allocate ActrId + credential
/// - `ActrToSignaling(Ping)` → respond with Pong
/// - `ActrToSignaling(RouteCandidatesRequest)` → return registered candidates
/// - `ActrRelay` → relay forwarding with RoleNegotiation handling
pub struct MockSignalingServer {
    port: u16,
    is_running: Arc<AtomicBool>,
    message_count: Arc<AtomicU32>,
    ice_restart_offer_count: Arc<AtomicU32>,
    pause_forwarding: Arc<AtomicBool>,
    connection_count: Arc<AtomicU32>,
    disconnection_count: Arc<AtomicU32>,
    #[allow(dead_code)]
    received_messages: Arc<Mutex<Vec<SignalingEnvelope>>>,
    cancel: tokio_util::sync::CancellationToken,
}

/// Shared state for the mock signaling server.
struct ServerState {
    /// WebSocket client senders, keyed by internal client_id.
    clients: RwLock<HashMap<String, mpsc::UnboundedSender<Message>>>,
    /// Registered actors, keyed by ActrId string representation.
    registry: RwLock<Vec<RegisteredActor>>,
    /// Mapping from client_id to ActrId (set after registration).
    client_to_actr_id: RwLock<HashMap<String, ActrId>>,
    /// Serial number allocator.
    next_serial: AtomicU64,
    /// Ed25519 signing key for generating credentials.
    signing_key: SigningKey,
    /// Key ID for the signing key.
    signing_key_id: u32,

    // Counters and controls
    message_count: Arc<AtomicU32>,
    ice_restart_offer_count: Arc<AtomicU32>,
    pause_forwarding: Arc<AtomicBool>,
    connection_count: Arc<AtomicU32>,
    disconnection_count: Arc<AtomicU32>,
    received_messages: Arc<Mutex<Vec<SignalingEnvelope>>>,
}

impl MockSignalingServer {
    /// Start the mock server on a random available port.
    pub async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        Self::start_with_listener(listener).await
    }

    /// Start the mock server on the specified port.
    pub async fn start_on_port(port: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
        Self::start_with_listener(listener).await
    }

    async fn start_with_listener(listener: TcpListener) -> anyhow::Result<Self> {
        let port = listener.local_addr()?.port();
        let message_count = Arc::new(AtomicU32::new(0));
        let ice_restart_offer_count = Arc::new(AtomicU32::new(0));
        let connection_count = Arc::new(AtomicU32::new(0));
        let disconnection_count = Arc::new(AtomicU32::new(0));
        let received_messages = Arc::new(Mutex::new(Vec::new()));
        let pause_forwarding = Arc::new(AtomicBool::new(false));
        let is_running = Arc::new(AtomicBool::new(true));
        let cancel = tokio_util::sync::CancellationToken::new();

        // Generate a deterministic signing key for tests
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let signing_key_id = 1u32;

        let state = Arc::new(ServerState {
            clients: RwLock::new(HashMap::new()),
            registry: RwLock::new(Vec::new()),
            client_to_actr_id: RwLock::new(HashMap::new()),
            next_serial: AtomicU64::new(1),
            signing_key,
            signing_key_id,
            message_count: message_count.clone(),
            ice_restart_offer_count: ice_restart_offer_count.clone(),
            pause_forwarding: pause_forwarding.clone(),
            connection_count: connection_count.clone(),
            disconnection_count: disconnection_count.clone(),
            received_messages: received_messages.clone(),
        });

        let is_running_clone = is_running.clone();
        let cancel_clone = cancel.clone();
        let (ready_tx, ready_rx) = oneshot::channel();

        tokio::spawn(async move {
            Self::run_server(listener, state, is_running_clone, cancel_clone, ready_tx).await;
        });

        // Use an in-process readiness signal so startup bookkeeping does not
        // create a synthetic TCP connection and pollute connection_count.
        tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("mock signaling server failed to start on port {port}"))?
            .map_err(|_| anyhow::anyhow!("mock signaling server startup task exited early"))?;

        Ok(Self {
            port,
            is_running,
            message_count,
            ice_restart_offer_count,
            received_messages,
            pause_forwarding,
            connection_count,
            disconnection_count,
            cancel,
        })
    }

    async fn run_server(
        listener: TcpListener,
        state: Arc<ServerState>,
        is_running: Arc<AtomicBool>,
        cancel: tokio_util::sync::CancellationToken,
        ready_tx: oneshot::Sender<()>,
    ) {
        let _ = ready_tx.send(());

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("mock signaling server shutting down");
                    is_running.store(false, Ordering::Release);
                    break;
                }

                accept_result = listener.accept() => {
                    if let Ok((stream, addr)) = accept_result {
                        tracing::debug!("mock signaling: new connection from {addr}");
                        state.connection_count.fetch_add(1, Ordering::SeqCst);

                        let state = state.clone();
                        let cancel = cancel.clone();

                        tokio::spawn(async move {
                            Self::handle_connection(stream, state, cancel).await;
                        });
                    }
                }
            }
        }
    }

    async fn handle_connection(
        stream: tokio::net::TcpStream,
        state: Arc<ServerState>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        let Ok(ws_stream) = accept_async(stream).await else {
            return;
        };

        let (mut ws_tx, mut ws_rx) = ws_stream.split();
        let (client_tx, mut client_rx) = mpsc::unbounded_channel();
        let client_id = uuid::Uuid::new_v4().to_string();

        state
            .clients
            .write()
            .await
            .insert(client_id.clone(), client_tx);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = ws_tx.send(Message::Close(None)).await;
                    break;
                }

                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Binary(data))) => {
                            state.message_count.fetch_add(1, Ordering::Relaxed);

                            if let Ok(envelope) = <SignalingEnvelope as ProstMessage>::decode(&data[..]) {
                                state.received_messages.lock().await.push(envelope.clone());
                                Self::process_envelope(envelope, &client_id, &state).await;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }

                msg = client_rx.recv() => {
                    if let Some(msg) = msg {
                        if ws_tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }

        // Cleanup
        state.clients.write().await.remove(&client_id);
        // Remove actor registration for this client
        {
            let mut registry = state.registry.write().await;
            registry.retain(|a| a.client_id != client_id);
        }
        state.client_to_actr_id.write().await.remove(&client_id);
        state.disconnection_count.fetch_add(1, Ordering::SeqCst);
    }

    async fn process_envelope(
        envelope: SignalingEnvelope,
        sender_id: &str,
        state: &Arc<ServerState>,
    ) {
        let Some(flow) = envelope.flow.as_ref() else {
            return;
        };

        match flow {
            signaling_envelope::Flow::PeerToServer(peer_msg) => {
                Self::handle_peer_to_server(&envelope, peer_msg, sender_id, state).await;
            }
            signaling_envelope::Flow::ActrToServer(actr_msg) => {
                Self::handle_actr_to_server(&envelope, actr_msg, sender_id, state).await;
            }
            signaling_envelope::Flow::ActrRelay(relay) => {
                Self::handle_actr_relay(&envelope, relay, sender_id, state).await;
            }
            _ => {}
        }
    }

    /// Handle PeerToSignaling messages (pre-registration).
    async fn handle_peer_to_server(
        envelope: &SignalingEnvelope,
        peer_msg: &actr_protocol::PeerToSignaling,
        sender_id: &str,
        state: &Arc<ServerState>,
    ) {
        let Some(payload) = peer_msg.payload.as_ref() else {
            return;
        };

        match payload {
            peer_to_signaling::Payload::RegisterRequest(req) => {
                let serial = state.next_serial.fetch_add(1, Ordering::SeqCst);

                let actr_id = ActrId {
                    realm: req.realm,
                    serial_number: serial,
                    r#type: req.actr_type.clone(),
                };

                // Generate credential using signing key
                let claims = actr_protocol::IdentityClaims {
                    realm_id: req.realm.realm_id,
                    actor_id: format!("{serial}"),
                    expires_at: chrono::Utc::now().timestamp() as u64 + 86400,
                };
                let claims_bytes = claims.encode_to_vec();

                use ed25519_dalek::Signer;
                let signature = state.signing_key.sign(&claims_bytes);
                let verifying_key = state.signing_key.verifying_key();

                let credential = AIdCredential {
                    key_id: state.signing_key_id,
                    claims: claims_bytes.into(),
                    signature: signature.to_bytes().to_vec().into(),
                };

                let turn_credential = TurnCredential {
                    username: format!("{}:{}", chrono::Utc::now().timestamp() + 86400, serial),
                    password: base64_encode_turn_password(serial),
                    expires_at: chrono::Utc::now().timestamp() as u64 + 86400,
                };

                // Store registration
                {
                    let entry = RegisteredActor {
                        actr_id: actr_id.clone(),
                        actr_type: req.actr_type.clone(),
                        client_id: sender_id.to_string(),
                        ws_address: req.ws_address.clone(),
                        service_spec: req.service_spec.clone(),
                    };
                    state.registry.write().await.push(entry);
                    state
                        .client_to_actr_id
                        .write()
                        .await
                        .insert(sender_id.to_string(), actr_id.clone());
                }

                let register_ok = register_response::RegisterOk {
                    actr_id: actr_id.clone(),
                    credential,
                    turn_credential,
                    credential_expires_at: Some(prost_types::Timestamp {
                        seconds: chrono::Utc::now().timestamp() + 86400,
                        nanos: 0,
                    }),
                    signaling_heartbeat_interval_secs: 30,
                    signing_pubkey: verifying_key.to_bytes().to_vec().into(),
                    signing_key_id: state.signing_key_id,
                    psk: None,
                    psk_expires_at: None,
                };

                let response = RegisterResponse {
                    result: Some(register_response::Result::Success(register_ok)),
                };

                let response_envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: Some(envelope.envelope_id.clone()),
                    timestamp: now_timestamp(),
                    flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                        target: actr_id,
                        payload: Some(signaling_to_actr::Payload::RegisterResponse(response)),
                    })),
                    traceparent: None,
                    tracestate: None,
                };

                send_to_client(sender_id, &response_envelope, &state.clients).await;

                tracing::info!(
                    serial,
                    manufacturer = req.actr_type.manufacturer,
                    name = req.actr_type.name,
                    "mock signaling: registered actor"
                );
            }
        }
    }

    /// Handle ActrToSignaling messages (authenticated session).
    async fn handle_actr_to_server(
        envelope: &SignalingEnvelope,
        actr_msg: &actr_protocol::ActrToSignaling,
        sender_id: &str,
        state: &Arc<ServerState>,
    ) {
        let Some(payload) = actr_msg.payload.as_ref() else {
            return;
        };

        match payload {
            actr_to_signaling::Payload::Ping(_ping) => {
                let pong = actr_protocol::Pong {
                    seq: state.message_count.load(Ordering::Relaxed) as u64,
                    suggest_interval_secs: Some(30),
                    credential_warning: None,
                };

                let response_envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: Some(envelope.envelope_id.clone()),
                    timestamp: now_timestamp(),
                    flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                        target: actr_msg.source.clone(),
                        payload: Some(signaling_to_actr::Payload::Pong(pong)),
                    })),
                    traceparent: None,
                    tracestate: None,
                };

                send_to_client(sender_id, &response_envelope, &state.clients).await;
            }

            actr_to_signaling::Payload::RouteCandidatesRequest(req) => {
                let registry = state.registry.read().await;

                // Find candidates matching the requested type
                let mut candidates = Vec::new();
                let mut ws_address_map = Vec::new();

                for entry in registry.iter() {
                    if entry.actr_type == req.target_type {
                        // Skip self
                        if entry.actr_id.serial_number == actr_msg.source.serial_number {
                            continue;
                        }

                        candidates.push(entry.actr_id.clone());

                        if let Some(ws_addr) = &entry.ws_address {
                            ws_address_map.push(actr_protocol::WsAddressEntry {
                                candidate_id: entry.actr_id.clone(),
                                ws_address: Some(ws_addr.clone()),
                            });
                        }
                    }
                }

                // Limit by requested count
                if let Some(criteria) = &req.criteria {
                    let max = criteria.candidate_count as usize;
                    candidates.truncate(max);
                }

                tracing::info!(
                    count = candidates.len(),
                    target_type =
                        format!("{}.{}", req.target_type.manufacturer, req.target_type.name),
                    "mock signaling: route candidates response"
                );

                let ok = route_candidates_response::RouteCandidatesOk {
                    candidates,
                    ws_address_map,
                };

                let response = RouteCandidatesResponse {
                    result: Some(route_candidates_response::Result::Success(ok)),
                };

                let response_envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: Some(envelope.envelope_id.clone()),
                    timestamp: now_timestamp(),
                    flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                        target: actr_msg.source.clone(),
                        payload: Some(signaling_to_actr::Payload::RouteCandidatesResponse(
                            response,
                        )),
                    })),
                    traceparent: None,
                    tracestate: None,
                };

                send_to_client(sender_id, &response_envelope, &state.clients).await;
            }

            actr_to_signaling::Payload::UnregisterRequest(_) => {
                // Remove from registry
                {
                    let mut registry = state.registry.write().await;
                    registry.retain(|a| a.client_id != sender_id);
                }

                let response_envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: Some(envelope.envelope_id.clone()),
                    timestamp: now_timestamp(),
                    flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                        target: actr_msg.source.clone(),
                        payload: Some(signaling_to_actr::Payload::UnregisterResponse(
                            actr_protocol::UnregisterResponse {
                                result: Some(actr_protocol::unregister_response::Result::Success(
                                    actr_protocol::unregister_response::UnregisterOk {},
                                )),
                            },
                        )),
                    })),
                    traceparent: None,
                    tracestate: None,
                };

                send_to_client(sender_id, &response_envelope, &state.clients).await;
            }

            actr_to_signaling::Payload::GetSigningKeyRequest(req) => {
                let verifying_key = state.signing_key.verifying_key();
                let response_envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: Some(envelope.envelope_id.clone()),
                    timestamp: now_timestamp(),
                    flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                        target: actr_msg.source.clone(),
                        payload: Some(signaling_to_actr::Payload::GetSigningKeyResponse(
                            actr_protocol::GetSigningKeyResponse {
                                key_id: req.key_id,
                                pubkey: verifying_key.to_bytes().to_vec().into(),
                            },
                        )),
                    })),
                    traceparent: None,
                    tracestate: None,
                };

                send_to_client(sender_id, &response_envelope, &state.clients).await;
            }

            actr_to_signaling::Payload::DiscoveryRequest(req) => {
                let registry = state.registry.read().await;

                let entries: Vec<_> = registry
                    .iter()
                    .filter(|e| {
                        req.manufacturer
                            .as_ref()
                            .is_none_or(|m| &e.actr_type.manufacturer == m)
                    })
                    .map(|e| {
                        let fingerprint = e
                            .service_spec
                            .as_ref()
                            .map_or(String::new(), |s| s.fingerprint.clone());
                        actr_protocol::discovery_response::TypeEntry {
                            actr_type: e.actr_type.clone(),
                            name: e
                                .service_spec
                                .as_ref()
                                .map_or(e.actr_type.name.clone(), |s| s.name.clone()),
                            description: e
                                .service_spec
                                .as_ref()
                                .and_then(|s| s.description.clone()),
                            service_fingerprint: fingerprint,
                            published_at: e.service_spec.as_ref().and_then(|s| s.published_at),
                            tags: e.service_spec.as_ref().map_or(vec![], |s| s.tags.clone()),
                        }
                    })
                    .collect();

                let response_envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: Some(envelope.envelope_id.clone()),
                    timestamp: now_timestamp(),
                    flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                        target: actr_msg.source.clone(),
                        payload: Some(signaling_to_actr::Payload::DiscoveryResponse(
                            actr_protocol::DiscoveryResponse {
                                result: Some(actr_protocol::discovery_response::Result::Success(
                                    actr_protocol::discovery_response::DiscoveryOk { entries },
                                )),
                            },
                        )),
                    })),
                    traceparent: None,
                    tracestate: None,
                };

                send_to_client(sender_id, &response_envelope, &state.clients).await;
            }

            actr_to_signaling::Payload::GetServiceSpecRequest(req) => {
                let registry = state.registry.read().await;

                // Look up service spec by name
                let spec = registry.iter().find_map(|e| {
                    if let Some(spec) = &e.service_spec {
                        if spec.name == req.name {
                            return Some(spec.clone());
                        }
                    }
                    None
                });

                let payload = match spec {
                    Some(spec) => signaling_to_actr::Payload::GetServiceSpecResponse(
                        actr_protocol::GetServiceSpecResponse {
                            result: Some(
                                actr_protocol::get_service_spec_response::Result::Success(spec),
                            ),
                        },
                    ),
                    None => signaling_to_actr::Payload::GetServiceSpecResponse(
                        actr_protocol::GetServiceSpecResponse {
                            result: Some(actr_protocol::get_service_spec_response::Result::Error(
                                actr_protocol::ErrorResponse {
                                    code: 404,
                                    message: format!("service '{}' not found", req.name),
                                },
                            )),
                        },
                    ),
                };

                let response_envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: Some(envelope.envelope_id.clone()),
                    timestamp: now_timestamp(),
                    flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                        target: actr_msg.source.clone(),
                        payload: Some(payload),
                    })),
                    traceparent: None,
                    tracestate: None,
                };

                send_to_client(sender_id, &response_envelope, &state.clients).await;
            }

            // Other payloads: silently ignored
            _ => {
                tracing::debug!("mock signaling: ignoring ActrToSignaling payload");
            }
        }
    }

    /// Handle ActrRelay messages (peer-to-peer relay).
    async fn handle_actr_relay(
        envelope: &SignalingEnvelope,
        relay: &ActrRelay,
        sender_id: &str,
        state: &Arc<ServerState>,
    ) {
        // Track ICE restart offers
        if let Some(actr_relay::Payload::SessionDescription(sd)) = relay.payload.as_ref() {
            if sd.r#type == 3 {
                // IceRestartOffer
                state.ice_restart_offer_count.fetch_add(1, Ordering::SeqCst);
            }
        }

        if state.pause_forwarding.load(Ordering::Acquire) {
            return;
        }

        // Handle RoleNegotiation: server decides roles
        if let Some(actr_relay::Payload::RoleNegotiation(role_neg)) = relay.payload.as_ref() {
            let from_is_offerer = role_neg.from.serial_number < role_neg.to.serial_number;

            let envelope_for_from = SignalingEnvelope {
                envelope_version: 1,
                envelope_id: uuid::Uuid::new_v4().to_string(),
                reply_for: None,
                timestamp: now_timestamp(),
                flow: Some(signaling_envelope::Flow::ActrRelay(ActrRelay {
                    source: role_neg.to.clone(),
                    credential: AIdCredential::default(),
                    target: role_neg.from.clone(),
                    payload: Some(actr_relay::Payload::RoleAssignment(
                        actr_protocol::RoleAssignment {
                            is_offerer: from_is_offerer,
                            remote_fixed: None,
                        },
                    )),
                })),
                traceparent: None,
                tracestate: None,
            };

            let envelope_for_to = SignalingEnvelope {
                envelope_version: 1,
                envelope_id: uuid::Uuid::new_v4().to_string(),
                reply_for: None,
                timestamp: now_timestamp(),
                flow: Some(signaling_envelope::Flow::ActrRelay(ActrRelay {
                    source: role_neg.from.clone(),
                    credential: AIdCredential::default(),
                    target: role_neg.to.clone(),
                    payload: Some(actr_relay::Payload::RoleAssignment(
                        actr_protocol::RoleAssignment {
                            is_offerer: !from_is_offerer,
                            remote_fixed: None,
                        },
                    )),
                })),
                traceparent: None,
                tracestate: None,
            };

            // Route by ActrId
            let clients = state.clients.read().await;

            // Find client_id for `from` and `to`
            for (cid, tx) in clients.iter() {
                if cid == sender_id {
                    let encoded = envelope_for_from.encode_to_vec();
                    let _ = tx.send(Message::Binary(encoded.into()));
                } else {
                    let encoded = envelope_for_to.encode_to_vec();
                    let _ = tx.send(Message::Binary(encoded.into()));
                }
            }
            return;
        }

        // Forward relay to target by ActrId lookup
        let target_id = &relay.target;
        let client_map = state.client_to_actr_id.read().await;
        let clients = state.clients.read().await;

        // Find the client_id for the target ActrId
        let target_client_id = client_map.iter().find_map(|(cid, aid)| {
            if aid == target_id {
                Some(cid.clone())
            } else {
                None
            }
        });

        if let Some(target_cid) = target_client_id {
            if let Some(tx) = clients.get(&target_cid) {
                let encoded = envelope.encode_to_vec();
                let _ = tx.send(Message::Binary(encoded.into()));
            }
        } else {
            // Fallback: broadcast to all other clients
            let encoded = envelope.encode_to_vec();
            for (cid, tx) in clients.iter() {
                if cid != sender_id {
                    let _ = tx.send(Message::Binary(encoded.clone().into()));
                }
            }
        }
    }

    /// Get server WebSocket URL.
    pub fn url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Get server port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Shutdown the server.
    pub async fn shutdown(&mut self) {
        if !self.cancel.is_cancelled() {
            self.cancel.cancel();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Pause message forwarding (relay messages are dropped).
    pub fn pause_forwarding(&self) {
        self.pause_forwarding.store(true, Ordering::Release);
    }

    /// Resume message forwarding.
    pub fn resume_forwarding(&self) {
        self.pause_forwarding.store(false, Ordering::Release);
    }

    /// Get total message count.
    pub fn message_count(&self) -> u32 {
        self.message_count.load(Ordering::Relaxed)
    }

    /// Get ICE restart offer count.
    pub fn ice_restart_count(&self) -> u32 {
        self.ice_restart_offer_count.load(Ordering::SeqCst)
    }

    /// Get connection count.
    pub fn connection_count(&self) -> u32 {
        self.connection_count.load(Ordering::SeqCst)
    }

    /// Get disconnection count.
    pub fn disconnection_count(&self) -> u32 {
        self.disconnection_count.load(Ordering::SeqCst)
    }

    /// Reset all counters.
    pub fn reset_counters(&self) {
        self.message_count.store(0, Ordering::Relaxed);
        self.ice_restart_offer_count.store(0, Ordering::SeqCst);
        self.connection_count.store(0, Ordering::SeqCst);
        self.disconnection_count.store(0, Ordering::SeqCst);
    }

    /// Check if server is running.
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Acquire)
    }
}

impl Drop for MockSignalingServer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_timestamp() -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: chrono::Utc::now().timestamp(),
        nanos: 0,
    }
}

async fn send_to_client(
    client_id: &str,
    envelope: &SignalingEnvelope,
    clients: &RwLock<HashMap<String, mpsc::UnboundedSender<Message>>>,
) {
    let encoded = envelope.encode_to_vec();
    let clients_read = clients.read().await;
    if let Some(tx) = clients_read.get(client_id) {
        let _ = tx.send(Message::Binary(encoded.into()));
    }
}

fn base64_encode_turn_password(serial: u64) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = write!(s, "mock-turn-{serial:016x}");
    s
}
