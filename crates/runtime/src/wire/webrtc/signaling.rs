//! signaling clientImplementation
//!
//! Based on protobuf Definition'ssignalingprotocol, using SignalingEnvelope conclude construct

#[cfg(feature = "opentelemetry")]
use super::trace;
use crate::transport::error::{NetworkError, NetworkResult};
#[cfg(feature = "opentelemetry")]
use crate::wire::webrtc::trace::extract_trace_context;
#[cfg(feature = "opentelemetry")]
use actr_protocol::ActrIdExt;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrToSignaling, PeerToSignaling, Ping, RegisterRequest,
    RegisterResponse, RouteCandidatesRequest, RouteCandidatesResponse, ServiceAvailabilityState,
    SignalingEnvelope, UnregisterRequest, UnregisterResponse, actr_to_signaling, peer_to_signaling,
    signaling_envelope, signaling_to_actr,
};
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};
use tokio_util::sync::CancellationToken;
#[cfg(feature = "opentelemetry")]
use tracing::instrument;
#[cfg(feature = "opentelemetry")]
use tracing_opentelemetry::OpenTelemetrySpanExt;
use url::Url;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Default timeout in seconds for waiting for signaling response
const RESPONSE_TIMEOUT_SECS: u64 = 15;
// WebSocket-level keepalive to detect silent half-open connections
const PING_INTERVAL_SECS: u64 = 5;
const PONG_TIMEOUT_SECS: u64 = 10;

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
    /// Subscribe to connection state changes (Connected/Disconnected).
    fn subscribe_state(&self) -> watch::Receiver<ConnectionState>;

    /// Set identity used for reconnect URL parameters (actor_id + credential).
    async fn set_reconnect_identity(&self, actor_id: ActrId, credential: AIdCredential);
}

/// High-level signaling connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connected,
}

/// WebSocket signaling clientImplementation
pub struct WebSocketSignalingClient {
    config: SignalingConfig,
    /// Reconnect identity (actor_id + credential) to attach as URL params on reconnects
    reconnect_identity: tokio::sync::Mutex<Option<(ActrId, AIdCredential)>>,
    /// WebSocket write end （using Mutex Implementation interior mutability ）
    ws_sink: Arc<
        tokio::sync::Mutex<
            Option<
                futures_util::stream::SplitSink<
                    WebSocketStream<MaybeTlsStream<TcpStream>>,
                    tokio_tungstenite::tungstenite::Message,
                >,
            >,
        >,
    >,
    /// WebSocket read end （using Mutex Implementation interior mutability ）
    ws_stream: tokio::sync::Mutex<
        Option<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
    >,
    /// connection status
    connected: Arc<AtomicBool>,
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
    receiver_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Background ping task to detect half-open connections
    ping_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Connection state broadcast channel
    state_tx: watch::Sender<ConnectionState>,
    /// Last time we saw inbound traffic (pong/any message), unix epoch seconds
    last_pong: Arc<AtomicU64>,
}

impl WebSocketSignalingClient {
    /// Create new WebSocket signaling client
    pub fn new(config: SignalingConfig) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        let (state_tx, _state_rx) = watch::channel(ConnectionState::Disconnected);
        Self {
            config,
            ws_sink: Arc::new(tokio::sync::Mutex::new(None)),
            ws_stream: tokio::sync::Mutex::new(None),
            connected: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(AtomicSignalingStats::default()),
            envelope_counter: tokio::sync::Mutex::new(0),
            pending_replies: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            inbound_rx: Arc::new(tokio::sync::Mutex::new(inbound_rx)),
            inbound_tx: tokio::sync::Mutex::new(inbound_tx),
            receiver_task: Arc::new(tokio::sync::Mutex::new(None)),
            ping_task: tokio::sync::Mutex::new(None),
            state_tx,
            last_pong: Arc::new(AtomicU64::new(0)),
            reconnect_identity: tokio::sync::Mutex::new(None),
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
            traceparent: None,
            tracestate: None,
            flow: Some(flow),
        }
    }

    /// Reset inbound channel for a fresh session (useful after disconnects).
    async fn reset_inbound_channel(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.inbound_tx.lock().await = tx;
        *self.inbound_rx.lock().await = rx;
    }

    /// Build signaling URL, attaching actor identity/token if available for reconnects.
    async fn build_url_with_identity(&self) -> Url {
        let mut url = self.config.server_url.clone();
        if let Some((actor_id, credential)) = self.reconnect_identity.lock().await.clone() {
            let actor_str = actr_protocol::ActrIdExt::to_string_repr(&actor_id);
            let token_b64 =
                base64::engine::general_purpose::STANDARD.encode(&credential.encrypted_token);
            {
                let mut pairs = url.query_pairs_mut();
                pairs.append_pair("actor_id", &actor_str);
                pairs.append_pair("token", &token_b64);
                pairs.append_pair("token_key_id", &credential.token_key_id.to_string());
            }
        }
        url
    }

    /// Establish a single signaling WebSocket connection attempt, honoring connection_timeout.
    ///
    /// This does not perform any retry logic; callers that want retries should wrap this.
    async fn establish_connection_once(&self) -> NetworkResult<()> {
        let url = self.build_url_with_identity().await;
        let timeout_secs = self.config.connection_timeout;
        tracing::debug!("Establishing connection to URL: {}", url.as_str());
        // 断网后，写入到缓冲区的数据，网络恢复后会继续发送
        let config = WebSocketConfig::default().write_buffer_size(0);
        // Connect with an optional timeout. A timeout of 0 means "no timeout".
        let connect_result = if timeout_secs == 0 {
            connect_async_with_config(url.as_str(), Some(config), false).await
        } else {
            let timeout_duration = std::time::Duration::from_secs(timeout_secs);
            tokio::time::timeout(
                timeout_duration,
                connect_async_with_config(url.as_str(), Some(config), false),
            )
            .await
            .map_err(|_| {
                NetworkError::ConnectionError(format!(
                    "Signaling connect timeout after {}s",
                    timeout_secs
                ))
            })?
        }?;

        let (ws_stream, _) = connect_result;

        // Split read/write halves and initialize client state
        let (sink, stream) = ws_stream.split();

        *self.ws_sink.lock().await = Some(sink);
        *self.ws_stream.lock().await = Some(stream);
        self.connected.store(true, Ordering::Release);
        self.last_pong.store(current_unix_secs(), Ordering::Release);
        // Notify listeners that we are now connected
        let _ = self.state_tx.send(ConnectionState::Connected);

        self.stats.connections.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Connect to signaling server with retry and exponential backoff based on reconnect_config.
    async fn connect_with_retries(&self) -> NetworkResult<()> {
        let cfg = &self.config.reconnect_config;

        // If reconnect is disabled, just attempt once.
        if !cfg.enabled {
            return self.establish_connection_once().await;
        }

        let mut attempt: u32 = 0;
        let mut delay_secs = cfg.initial_delay.max(1);

        loop {
            attempt += 1;

            match self.establish_connection_once().await {
                Ok(()) => {
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Signaling connect attempt {} failed: {e:?}", attempt);

                    if attempt >= cfg.max_attempts {
                        tracing::error!(
                            "Signaling connect failed after {} attempts, giving up",
                            attempt
                        );
                        return Err(e);
                    }

                    let sleep_secs = delay_secs.min(cfg.max_delay.max(1));
                    tracing::info!("Retry signaling connect after {}s", sleep_secs);
                    tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;

                    // Exponential backoff for next attempt
                    delay_secs = ((delay_secs as f64) * cfg.backoff_multiplier)
                        .round()
                        .max(1.0) as u64;
                }
            }
        }
    }

    /// Send envelope and wait for response with timeout and error handling.
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, fields(envelope_id = %envelope.envelope_id))
    )]
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
        let connected = self.connected.clone();
        let state_tx = self.state_tx.clone();
        let last_pong = self.last_pong.clone();
        tracing::debug!("Start receiver");
        let handle = tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                        // Any inbound traffic counts as liveness
                        last_pong.store(current_unix_secs(), Ordering::Release);
                        match SignalingEnvelope::decode(&data[..]) {
                            Ok(envelope) => {
                                #[cfg(feature = "opentelemetry")]
                                let span = {
                                    let span = tracing::info_span!("signaling.receive_envelope", envelope_id = %envelope.envelope_id);
                                    span.set_parent(extract_trace_context(&envelope));
                                    span
                                };

                                stats.messages_received.fetch_add(1, Ordering::Relaxed);
                                tracing::debug!("Received message: {:?}", envelope);
                                if let Some(reply_for) = envelope.reply_for.clone() {
                                    if let Some(sender) = pending.lock().await.remove(&reply_for) {
                                        #[cfg(feature = "opentelemetry")]
                                        let _ = span.enter();
                                        if let Err(e) = sender.send(envelope) {
                                            stats.errors.fetch_add(1, Ordering::Relaxed);
                                            tracing::warn!(
                                                "Failed to send reply envelope to waiter: {e:?}",
                                            );
                                        }
                                        continue;
                                    }
                                }
                                tracing::debug!(
                                    "Unmatched or push message -> forward to inbound channel"
                                );
                                // Unmatched or push message -> forward to inbound channel
                                if let Err(e) = inbound_tx.send(envelope) {
                                    stats.errors.fetch_add(1, Ordering::Relaxed);
                                    tracing::warn!(
                                        "Failed to send envelope to inbound channel: {e:?}"
                                    );
                                }
                            }
                            Err(e) => {
                                stats.errors.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!("Failed to decode SignalingEnvelope: {e}");
                            }
                        }
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Pong(_)) => {
                        tracing::debug!("Received pong");
                        last_pong.store(current_unix_secs(), Ordering::Release);
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Ping(_)) => {
                        tracing::debug!("Received ping");
                        last_pong.store(current_unix_secs(), Ordering::Release);
                    }
                    Ok(_) => {
                        tracing::warn!("Received non-binary frame, ignoring");
                    }
                    Err(e) => {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("Signaling receive error: {e}");
                        break;
                    }
                }
            }

            // Reaching here means the underlying WebSocket stream has terminated.
            connected.store(false, Ordering::Release);
            stats.disconnections.fetch_add(1, Ordering::Relaxed);
            let _ = state_tx.send(ConnectionState::Disconnected);
        });

        *self.receiver_task.lock().await = Some(handle);
    }

    /// Spawn background ping task to detect half-open connections where writes do not fail but peer is gone.
    async fn start_ping_task(&self) {
        let mut existing = self.ping_task.lock().await;
        if let Some(handle) = existing.as_ref() {
            if handle.is_finished() {
                existing.take();
            } else {
                return;
            }
        }

        let sink = self.ws_sink.clone();
        let connected = self.connected.clone();
        let state_tx = self.state_tx.clone();
        let last_pong = self.last_pong.clone();
        let receiver_task_clone = Arc::clone(&self.receiver_task);

        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(PING_INTERVAL_SECS)).await;

                if !connected.load(Ordering::Acquire) {
                    break;
                }

                // Send ping; mark disconnect on failure.
                let mut sink_guard = sink.lock().await;
                if let Some(sink) = sink_guard.as_mut() {
                    if let Err(e) = sink
                        .send(tokio_tungstenite::tungstenite::Message::Ping(
                            Vec::new().into(),
                        ))
                        .await
                    {
                        tracing::warn!("Signaling ping send failed: {e}");
                        connected.store(false, Ordering::Release);
                        let _ = state_tx.send(ConnectionState::Disconnected);
                        break;
                    }
                } else {
                    tracing::warn!("Signaling not connected");
                    connected.store(false, Ordering::Release);
                    let _ = state_tx.send(ConnectionState::Disconnected);
                    break;
                }
                drop(sink_guard);

                // Check for stale pong
                let now = current_unix_secs();
                let last = last_pong.load(Ordering::Acquire);
                if now.saturating_sub(last) > PONG_TIMEOUT_SECS {
                    tracing::warn!(
                        "Signaling pong timeout (last seen {}s ago), marking disconnected",
                        now.saturating_sub(last)
                    );
                    connected.store(false, Ordering::Release);
                    let _ = state_tx.send(ConnectionState::Disconnected);
                    if let Some(handle) = receiver_task_clone.lock().await.take() {
                        handle.abort();
                    }
                    break;
                }
            }
        });

        *existing = Some(handle);
    }
}

#[async_trait]
impl SignalingClient for WebSocketSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        self.connect_with_retries().await?;
        self.start_receiver().await;
        self.start_ping_task().await;
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
        // Stop ping task if running
        if let Some(handle) = self.ping_task.lock().await.take() {
            handle.abort();
        }

        self.reset_inbound_channel().await;

        self.connected.store(false, Ordering::Release);
        self.stats.disconnections.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
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

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, fields(actor_id = %actor_id.to_string_repr()))
    )]
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

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "debug", skip_all, fields(actor_id = %actor_id.to_string_repr()))
    )]
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

    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
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

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "debug", skip_all, fields(envelope_id = %envelope.envelope_id))
    )]
    async fn send_envelope(&self, envelope: SignalingEnvelope) -> NetworkResult<()> {
        #[cfg(feature = "opentelemetry")]
        let envelope = {
            let mut envelope = envelope;
            trace::inject_span_context(&tracing::Span::current(), &mut envelope);
            envelope
        };

        let mut sink_guard = self.ws_sink.lock().await;

        if let Some(sink) = sink_guard.as_mut() {
            // using protobuf binary serialization
            let mut buf = Vec::new();
            envelope.encode(&mut buf)?;
            let msg = tokio_tungstenite::tungstenite::Message::Binary(buf.into());
            sink.send(msg).await?;

            self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);
            tracing::debug!("Stats: {:?}", self.stats.snapshot());
            Ok(())
        } else {
            Err(NetworkError::ConnectionError("Not connected".to_string()))
        }
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        let mut rx = self.inbound_rx.lock().await;
        match rx.recv().await {
            Some(envelope) => Ok(Some(envelope)),
            None => {
                tracing::error!("Inbound channel closed");
                Err(NetworkError::ConnectionError(
                    "Inbound channel closed".to_string(),
                ))
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn get_stats(&self) -> SignalingStats {
        self.stats.snapshot()
    }

    fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_tx.subscribe()
    }

    async fn set_reconnect_identity(&self, actor_id: ActrId, credential: AIdCredential) {
        let mut guard = self.reconnect_identity.lock().await;
        *guard = Some((actor_id, credential));
    }
}

/// Spawn a helper task that listens to connection state changes and proactively reconnects
/// when the signaling client transitions to Disconnected.
pub fn spawn_signaling_reconnector(client: Arc<dyn SignalingClient>, shutdown: CancellationToken) {
    let mut state_rx = client.subscribe_state();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("🛑 Stopping signaling reconnect helper due to shutdown");
                    break;
                }
                changed = state_rx.changed() => {
                    if changed.is_err() {
                        tracing::info!("Signaling state channel closed, stopping reconnect helper");
                        break;
                    }

                    if *state_rx.borrow() == ConnectionState::Disconnected {
                        // Double-check shutdown state to avoid reconnecting after shutdown
                        if shutdown.is_cancelled() {
                            tracing::info!(
                                "Shutdown already requested when disconnect event observed; skipping reconnect"
                            );
                            break;
                        }

                        tracing::warn!("📡 Signaling state is DISCONNECTED, attempting reconnect");
                        if let Err(e) = client.connect().await {
                            tracing::error!("❌ Signaling reconnect failed: {e}");
                        } else {
                            tracing::info!("✅ Signaling reconnect succeeded");
                        }

                    }
                }
            }
        }
    });
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

fn current_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as UsizeOrdering};
    use tokio_util::sync::CancellationToken;

    /// Simple fake SignalingClient implementation for testing the reconnect helper.
    struct FakeSignalingClient {
        state_tx: watch::Sender<ConnectionState>,
        connect_calls: Arc<AtomicUsize>,
        reconnect_identity: tokio::sync::Mutex<Option<(ActrId, AIdCredential)>>,
    }

    #[async_trait]
    impl SignalingClient for FakeSignalingClient {
        async fn connect(&self) -> NetworkResult<()> {
            self.connect_calls.fetch_add(1, UsizeOrdering::SeqCst);
            Ok(())
        }

        async fn disconnect(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn send_register_request(
            &self,
            _request: RegisterRequest,
        ) -> NetworkResult<RegisterResponse> {
            unimplemented!("not needed in tests");
        }

        async fn send_unregister_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _reason: Option<String>,
        ) -> NetworkResult<UnregisterResponse> {
            unimplemented!("not needed in tests");
        }

        async fn send_heartbeat(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _availability: ServiceAvailabilityState,
            _power_reserve: f32,
            _mailbox_backlog: f32,
        ) -> NetworkResult<()> {
            unimplemented!("not needed in tests");
        }

        async fn send_route_candidates_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _request: RouteCandidatesRequest,
        ) -> NetworkResult<RouteCandidatesResponse> {
            unimplemented!("not needed in tests");
        }

        async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
            unimplemented!("not needed in tests");
        }

        async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
            unimplemented!("not needed in tests");
        }

        fn is_connected(&self) -> bool {
            // Derived from last published state; keep implementation simple for tests.
            *self.state_tx.borrow() == ConnectionState::Connected
        }

        fn get_stats(&self) -> SignalingStats {
            SignalingStats::default()
        }

        fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
            self.state_tx.subscribe()
        }

        async fn set_reconnect_identity(&self, actor_id: ActrId, credential: AIdCredential) {
            let mut guard = self.reconnect_identity.lock().await;
            *guard = Some((actor_id, credential));
        }
    }

    fn make_fake_client() -> (Arc<FakeSignalingClient>, watch::Sender<ConnectionState>) {
        let (state_tx, _rx) = watch::channel(ConnectionState::Disconnected);
        let client = Arc::new(FakeSignalingClient {
            state_tx: state_tx.clone(),
            connect_calls: Arc::new(AtomicUsize::new(0)),
            reconnect_identity: tokio::sync::Mutex::new(None),
        });
        (client, state_tx)
    }

    #[tokio::test]
    async fn test_spawn_signaling_reconnector_does_not_trigger_on_connected() {
        let (fake_client, state_tx) = make_fake_client();
        let shutdown = CancellationToken::new();

        // Upcast to trait object
        let client_trait: Arc<dyn SignalingClient> = fake_client.clone();
        spawn_signaling_reconnector(client_trait, shutdown.clone());

        // Send a Connected state (should not trigger reconnect)
        let _ = state_tx.send(ConnectionState::Connected);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let calls = fake_client.connect_calls.load(UsizeOrdering::SeqCst);
        assert_eq!(
            calls, 0,
            "connect() should not be called on Connected state"
        );

        shutdown.cancel();
    }

    #[tokio::test]
    async fn test_spawn_signaling_reconnector_triggers_connect_on_disconnect() {
        let (fake_client, state_tx) = make_fake_client();
        let shutdown = CancellationToken::new();

        // Upcast to trait object
        let client_trait: Arc<dyn SignalingClient> = fake_client.clone();
        spawn_signaling_reconnector(client_trait, shutdown.clone());

        // Simulate a disconnect event
        let _ = state_tx.send(ConnectionState::Disconnected);

        // Give the helper some time to observe the event and call connect()
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let calls = fake_client.connect_calls.load(UsizeOrdering::SeqCst);
        assert!(
            calls >= 1,
            "expected at least one reconnect attempt, got {}",
            calls
        );

        shutdown.cancel();
    }

    #[tokio::test]
    async fn test_spawn_signaling_reconnector_stops_on_shutdown_before_disconnect() {
        let (fake_client, state_tx) = make_fake_client();
        let shutdown = CancellationToken::new();

        let client_trait: Arc<dyn SignalingClient> = fake_client.clone();
        spawn_signaling_reconnector(client_trait, shutdown.clone());

        // Immediately signal shutdown
        shutdown.cancel();

        // Then send a disconnect event after shutdown
        let _ = state_tx.send(ConnectionState::Disconnected);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let calls = fake_client.connect_calls.load(UsizeOrdering::SeqCst);
        assert_eq!(calls, 0, "reconnect helper should not run after shutdown");
    }

    #[test]
    fn test_websocket_signaling_client_initial_state_disconnected() {
        // Build a minimal config; URL doesn't need to be reachable for this test.
        let config = SignalingConfig {
            server_url: Url::parse("ws://example.com/signaling/ws").unwrap(),
            connection_timeout: 30,
            heartbeat_interval: 30,
            reconnect_config: ReconnectConfig::default(),
            auth_config: None,
        };

        let client = WebSocketSignalingClient::new(config);
        let state_rx = client.subscribe_state();
        assert_eq!(*state_rx.borrow(), ConnectionState::Disconnected);
    }
}
