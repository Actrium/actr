//! signaling clientImplementation
//!
//! Based on protobuf Definition'ssignalingprotocol, using SignalingEnvelope conclude construct

use crate::transport::error::{NetworkError, NetworkResult};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrToSignaling, PeerToSignaling, Ping, RegisterRequest,
    RegisterResponse, RouteCandidatesRequest, RouteCandidatesResponse, ServiceAvailabilityState,
    SignalingEnvelope, UnregisterRequest, UnregisterResponse, actr_to_signaling, peer_to_signaling,
    signaling_envelope, signaling_to_actr,
};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use url::Url;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Default timeout in seconds for waiting for signaling response
const RESPONSE_TIMEOUT_SECS: u64 = 5;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// configurationType
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// signalingconfiguration
#[derive(Debug, Clone)]
pub struct SignalingConfig {
    /// signaling server URL
    pub server_url: Url,

    /// Connecttimeout temporal duration （seconds）
    pub connection_timeout: u64,

    /// center skipinterval（seconds）
    pub heartbeat_interval: u64,

    /// reconnection configuration
    pub reconnect_config: ReconnectConfig,

    /// acknowledge verify configuration
    pub auth_config: Option<AuthConfig>,
}

/// reconnection configuration
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// whether start usage automatic reconnection
    pub enabled: bool,

    /// maximum reconnection attempts
    pub max_attempts: u32,

    /// initial reconnection delay（seconds）
    pub initial_delay: u64,

    /// maximum reconnection delay（seconds）
    pub max_delay: u64,

    /// Backoff multiplier factor
    pub backoff_multiplier: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 10,
            initial_delay: 1,
            max_delay: 60,
            backoff_multiplier: 2.0,
        }
    }
}

/// acknowledge verify configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// acknowledge verify Type
    pub auth_type: AuthType,

    /// acknowledge verify credential data
    pub credentials: HashMap<String, String>,
}

/// acknowledge verify Type
#[derive(Debug, Clone)]
pub enum AuthType {
    /// no acknowledge verify
    None,
    /// Bearer Token
    BearerToken,
    /// API Key
    ApiKey,
    /// JWT
    Jwt,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Client interface and implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// signaling client connect port
///
/// # interior mutability
/// allMethodusing `&self` and non `&mut self`, with for conveniencein Arc in shared.
/// Implementation class needs interior mutability （ like Mutex）to manage WebSocket connection status.
#[async_trait]
pub trait SignalingClient: Send + Sync {
    /// Connecttosignaling server
    async fn connect(&self) -> NetworkResult<()>;

    /// DisconnectConnect
    async fn disconnect(&self) -> NetworkResult<()>;

    /// SendRegisterrequest（Register front stream process, using PeerToSignaling）
    async fn send_register_request(
        &self,
        request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse>;

    /// Send UnregisterRequest to signaling server (Actr → Signaling flow)
    ///
    /// This is used when an Actor is shutting down gracefully and wants to
    /// proactively notify the signaling server that it is no longer available.
    async fn send_unregister_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse>;

    /// Send center skip（Registerafter stream process, using ActrToSignaling）
    async fn send_heartbeat(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        availability: ServiceAvailabilityState,
        power_reserve: f32,
        mailbox_backlog: f32,
    ) -> NetworkResult<()>;

    /// Send RouteCandidatesRequest (requires authenticated Actor session)
    async fn send_route_candidates_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse>;

    /// Sendsignalingsignal seal （ pass usage Method）
    async fn send_envelope(&self, envelope: SignalingEnvelope) -> NetworkResult<()>;

    /// Receivesignalingsignal seal
    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>>;

    /// Check connection status
    fn is_connected(&self) -> bool;

    /// GetConnect statistics info
    fn get_stats(&self) -> SignalingStats;
}

/// WebSocket signaling clientImplementation
pub struct WebSocketSignalingClient {
    config: SignalingConfig,
    /// WebSocket write end （using Mutex Implementation interior mutability ）
    ws_sink: tokio::sync::Mutex<
        Option<
            futures_util::stream::SplitSink<
                WebSocketStream<MaybeTlsStream<TcpStream>>,
                tokio_tungstenite::tungstenite::Message,
            >,
        >,
    >,
    /// WebSocket read end （using Mutex Implementation interior mutability ）
    ws_stream: tokio::sync::Mutex<
        Option<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
    >,
    /// connection status
    connected: AtomicBool,
    /// statistics info
    stats: Arc<AtomicSignalingStats>,
    /// Envelope count number device
    envelope_counter: tokio::sync::Mutex<u64>,
    /// Pending reply waiters (reply_for -> oneshot)
    pending_replies: Arc<tokio::sync::Mutex<HashMap<String, oneshot::Sender<SignalingEnvelope>>>>,
    /// Inbound envelope channel for unmatched messages (ActrRelay / push)
    inbound_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<SignalingEnvelope>>>,
    inbound_tx: tokio::sync::Mutex<mpsc::UnboundedSender<SignalingEnvelope>>,
    /// Background receive task handle to allow graceful shutdown
    receiver_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl WebSocketSignalingClient {
    /// Create new WebSocket signaling client
    pub fn new(config: SignalingConfig) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        Self {
            config,
            ws_sink: tokio::sync::Mutex::new(None),
            ws_stream: tokio::sync::Mutex::new(None),
            connected: AtomicBool::new(false),
            stats: Arc::new(AtomicSignalingStats::default()),
            envelope_counter: tokio::sync::Mutex::new(0),
            pending_replies: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            inbound_rx: Arc::new(tokio::sync::Mutex::new(inbound_rx)),
            inbound_tx: tokio::sync::Mutex::new(inbound_tx),
            receiver_task: tokio::sync::Mutex::new(None),
        }
    }

    /// simple for convenience construct create Function
    pub async fn connect_to(url: &str) -> NetworkResult<Self> {
        let config = SignalingConfig {
            server_url: url.parse()?,
            connection_timeout: 30,
            heartbeat_interval: 30,
            reconnect_config: ReconnectConfig::default(),
            auth_config: None,
        };

        let client = Self::new(config);
        client.connect().await?;
        Ok(client)
    }

    /// alive integrate down a envelope ID
    async fn next_envelope_id(&self) -> String {
        let mut counter = self.envelope_counter.lock().await;
        *counter += 1;
        format!("env-{}", *counter)
    }

    /// Create SignalingEnvelope
    async fn create_envelope(&self, flow: signaling_envelope::Flow) -> SignalingEnvelope {
        SignalingEnvelope {
            envelope_version: 1,
            envelope_id: self.next_envelope_id().await,
            reply_for: None,
            timestamp: prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            },
            flow: Some(flow),
        }
    }

    /// Reset inbound channel for a fresh session (useful after disconnects).
    async fn reset_inbound_channel(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.inbound_tx.lock().await = tx;
        *self.inbound_rx.lock().await = rx;
    }

    /// Send envelope and wait for response with timeout and error handling.
    async fn send_envelope_and_wait_response(
        &self,
        envelope: SignalingEnvelope,
    ) -> NetworkResult<SignalingEnvelope> {
        let reply_for = envelope.envelope_id.clone();

        // Register waiter before sending
        let (tx, rx) = oneshot::channel();
        self.pending_replies
            .lock()
            .await
            .insert(reply_for.clone(), tx);

        if let Err(e) = self.send_envelope(envelope).await {
            // Cleanup waiter on immediate send failure to avoid leaks.
            self.pending_replies.lock().await.remove(&reply_for);
            return Err(e);
        }

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(RESPONSE_TIMEOUT_SECS), rx).await;
        // Clean up waiter on timeout
        if result.is_err() {
            self.pending_replies.lock().await.remove(&reply_for);
        }

        let response_envelope = result
            .map_err(|_| {
                NetworkError::ConnectionError(
                    "Timed out waiting for signaling response".to_string(),
                )
            })?
            .map_err(|_| {
                NetworkError::ConnectionError(
                    "Receiver dropped while waiting for signaling response".to_string(),
                )
            })?;

        Ok(response_envelope)
    }

    /// Spawn background receiver to demux envelopes by reply_for.
    async fn start_receiver(&self) {
        let mut stream_guard = self.ws_stream.lock().await;
        if stream_guard.is_none() {
            return;
        }

        let mut stream = stream_guard.take().expect("stream exists");
        let pending = self.pending_replies.clone();
        let inbound_tx = { self.inbound_tx.lock().await.clone() };
        let stats = self.stats.clone();

        let handle = tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                        match SignalingEnvelope::decode(&data[..]) {
                            Ok(envelope) => {
                                stats.messages_received.fetch_add(1, Ordering::Relaxed);

                                if let Some(reply_for) = envelope.reply_for.clone() {
                                    let mut pending_guard = pending.lock().await;
                                    if let Some(sender) = pending_guard.remove(&reply_for) {
                                        if let Err(e) = sender.send(envelope) {
                                            stats.errors.fetch_add(1, Ordering::Relaxed);
                                            tracing::warn!(
                                                "Failed to send reply envelope to waiter: {e:?}",
                                            );
                                        }
                                        continue;
                                    }
                                }

                                // Unmatched or push message -> forward to inbound channel
                                let _ = inbound_tx.send(envelope);
                            }
                            Err(e) => {
                                stats.errors.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!("Failed to decode SignalingEnvelope: {e}");
                            }
                        }
                    }
                    Ok(_) => {
                        tracing::warn!("Received non-binary frame, ignoring");
                    }
                    Err(e) => {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("Signaling receive error: {e}");
                    }
                }
            }
        });

        *self.receiver_task.lock().await = Some(handle);
    }
}

#[async_trait]
impl SignalingClient for WebSocketSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        let (ws_stream, _) = connect_async(self.config.server_url.as_str()).await?;

        // distribute apart read write stream
        let (sink, stream) = ws_stream.split();

        *self.ws_sink.lock().await = Some(sink);
        *self.ws_stream.lock().await = Some(stream);
        self.connected.store(true, Ordering::Release);

        // Ensure inbound channel is fresh for this session
        self.reset_inbound_channel().await;

        // Start background receiver to demux replies and push messages
        self.start_receiver().await;

        self.stats.connections.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        // fetch exit sink and stream
        let mut sink_guard = self.ws_sink.lock().await;
        let mut stream_guard = self.ws_stream.lock().await;

        // Close sink
        if let Some(mut sink) = sink_guard.take() {
            let _ = sink.close().await;
        }

        // clear blank stream
        stream_guard.take();

        // Stop receiver task if running
        if let Some(handle) = self.receiver_task.lock().await.take() {
            handle.abort();
        }

        self.reset_inbound_channel().await;

        self.connected.store(false, Ordering::Release);
        self.stats.disconnections.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    async fn send_register_request(
        &self,
        request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        // Create PeerToSignaling stream process （Register front ）
        let flow = signaling_envelope::Flow::PeerToServer(PeerToSignaling {
            payload: Some(peer_to_signaling::Payload::RegisterRequest(request)),
        });

        let envelope = self.create_envelope(flow).await;
        let response_envelope = self.send_envelope_and_wait_response(envelope).await?;

        if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) = response_envelope.flow
        {
            if let Some(signaling_to_actr::Payload::RegisterResponse(response)) =
                server_to_actr.payload
            {
                return Ok(response);
            }
        }

        Err(NetworkError::ConnectionError(
            "Invalid registration response".to_string(),
        ))
    }

    async fn send_unregister_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        // Build UnregisterRequest payload
        let request = UnregisterRequest {
            actr_id: actor_id.clone(),
            reason,
        };

        // Wrap into ActrToSignaling flow
        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::UnregisterRequest(request)),
        });

        // Send envelope (fire-and-forget)
        let envelope = self.create_envelope(flow).await;
        self.send_envelope(envelope).await?;

        // Do not wait for UnregisterResponse here because the signaling stream
        // is also consumed by WebRtcCoordinator. Waiting could race with that loop
        // and lead to spurious timeouts. Treat Unregister as best-effort.
        // not wait for the response , because the signaling stream have multi customers use it, fixme: should wait for the response
        Ok(UnregisterResponse {
            result: Some(actr_protocol::unregister_response::Result::Success(
                actr_protocol::unregister_response::UnregisterOk {},
            )),
        })
    }

    async fn send_heartbeat(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        availability: ServiceAvailabilityState,
        power_reserve: f32,
        mailbox_backlog: f32,
    ) -> NetworkResult<()> {
        let ping = Ping {
            availability: availability as i32,
            power_reserve,
            mailbox_backlog,
            sticky_client_ids: vec![], // TODO: Implement sticky session tracking
        };

        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::Ping(ping)),
        });

        let envelope = self.create_envelope(flow).await;
        self.send_envelope(envelope).await
    }

    async fn send_route_candidates_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::RouteCandidatesRequest(request)),
        });

        let envelope = self.create_envelope(flow).await;
        let response_envelope = self.send_envelope_and_wait_response(envelope).await?;

        if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) = response_envelope.flow
        {
            match server_to_actr.payload {
                Some(signaling_to_actr::Payload::RouteCandidatesResponse(response)) => {
                    return Ok(response);
                }
                Some(signaling_to_actr::Payload::Error(err)) => {
                    return Err(NetworkError::ServiceDiscoveryError(format!(
                        "{} ({})",
                        err.message, err.code
                    )));
                }
                _ => {}
            }
        }

        Err(NetworkError::ConnectionError(
            "Invalid route candidates response".to_string(),
        ))
    }

    async fn send_envelope(&self, envelope: SignalingEnvelope) -> NetworkResult<()> {
        let mut sink_guard = self.ws_sink.lock().await;

        if let Some(sink) = sink_guard.as_mut() {
            // using protobuf binary serialization
            let mut buf = Vec::new();
            envelope.encode(&mut buf)?;
            let msg = tokio_tungstenite::tungstenite::Message::Binary(buf.into());
            sink.send(msg).await?;

            self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);

            Ok(())
        } else {
            Err(NetworkError::ConnectionError("Not connected".to_string()))
        }
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        let mut rx = self.inbound_rx.lock().await;
        match rx.recv().await {
            Some(envelope) => Ok(Some(envelope)),
            None => Ok(None),
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn get_stats(&self) -> SignalingStats {
        self.stats.snapshot()
    }
}

/// signaling statistics info
#[derive(Debug)]
pub(crate) struct AtomicSignalingStats {
    /// Connect attempts
    pub connections: AtomicU64,

    /// DisconnectConnect attempts
    pub disconnections: AtomicU64,

    /// Send'smessage number
    pub messages_sent: AtomicU64,

    /// Receive'smessage number
    pub messages_received: AtomicU64,

    /// Send's center skip number
    /// TODO: Wire heartbeat counters when heartbeat send/receive paths are instrumented; currently never incremented.
    pub heartbeats_sent: AtomicU64,

    /// Receive's center skip number
    /// TODO: Wire heartbeat counters when heartbeat send/receive paths are instrumented; currently never incremented.
    pub heartbeats_received: AtomicU64,

    /// Error attempts
    pub errors: AtomicU64,
}

impl Default for AtomicSignalingStats {
    fn default() -> Self {
        Self {
            connections: AtomicU64::new(0),
            disconnections: AtomicU64::new(0),
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            heartbeats_sent: AtomicU64::new(0),
            heartbeats_received: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

/// Snapshot of statistics for serialization and reading
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SignalingStats {
    /// Connect attempts
    pub connections: u64,

    /// DisconnectConnect attempts
    pub disconnections: u64,

    /// Send'smessage number
    pub messages_sent: u64,

    /// Receive'smessage number
    pub messages_received: u64,

    /// Send's center skip number
    pub heartbeats_sent: u64,

    /// Receive's center skip number
    pub heartbeats_received: u64,

    /// Error attempts
    pub errors: u64,
}

impl AtomicSignalingStats {
    /// Create a snapshot of current statistics
    pub fn snapshot(&self) -> SignalingStats {
        SignalingStats {
            connections: self.connections.load(Ordering::Relaxed),
            disconnections: self.disconnections.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            heartbeats_sent: self.heartbeats_sent.load(Ordering::Relaxed),
            heartbeats_received: self.heartbeats_received.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}
