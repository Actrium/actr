//! signaling clientImplementation
//!
//! Based on protobuf Definition'ssignalingprotocol，using SignalingEnvelope conclude construct

use crate::transport::error::{NetworkError, NetworkResult};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrToSignaling, PeerToSignaling, Ping, RegisterRequest,
    RegisterResponse, ServiceAvailabilityState, SignalingEnvelope, actr_to_signaling,
    peer_to_signaling, signaling_envelope, signaling_to_actr,
};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use url::Url;

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
/// allMethodusing `&self` and non `&mut self`， with for conveniencein Arc in shared。
/// Implementation class needs interior mutability （ like Mutex）to manage WebSocket connection status。
#[async_trait]
pub trait SignalingClient: Send + Sync {
    /// Connecttosignaling server
    async fn connect(&self) -> NetworkResult<()>;

    /// DisconnectConnect
    async fn disconnect(&self) -> NetworkResult<()>;

    /// SendRegisterrequest（Register front stream process ，using PeerToSignaling）
    async fn send_register_request(
        &self,
        request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse>;

    /// Send center skip（Registerafter stream process ，using ActrToSignaling）
    async fn send_heartbeat(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        availability: ServiceAvailabilityState,
        power_reserve: f32,
        mailbox_backlog: f32,
    ) -> NetworkResult<()>;

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
    connected: tokio::sync::RwLock<bool>,
    /// statistics info
    stats: tokio::sync::RwLock<SignalingStats>,
    /// Envelope count number device
    envelope_counter: tokio::sync::Mutex<u64>,
}

impl WebSocketSignalingClient {
    /// Create new WebSocket signaling client
    pub fn new(config: SignalingConfig) -> Self {
        Self {
            config,
            ws_sink: tokio::sync::Mutex::new(None),
            ws_stream: tokio::sync::Mutex::new(None),
            connected: tokio::sync::RwLock::new(false),
            stats: tokio::sync::RwLock::new(SignalingStats::default()),
            envelope_counter: tokio::sync::Mutex::new(0),
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
}

#[async_trait]
impl SignalingClient for WebSocketSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        let (ws_stream, _) = connect_async(self.config.server_url.as_str()).await?;

        // distribute apart read write stream
        let (sink, stream) = ws_stream.split();

        *self.ws_sink.lock().await = Some(sink);
        *self.ws_stream.lock().await = Some(stream);
        *self.connected.write().await = true;

        let mut stats = self.stats.write().await;
        stats.connections += 1;

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

        *self.connected.write().await = false;

        let mut stats = self.stats.write().await;
        stats.disconnections += 1;

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
        self.send_envelope(envelope).await?;

        // Wait forRegisterresponse respond
        loop {
            if let Some(response_envelope) = self.receive_envelope().await? {
                if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) =
                    response_envelope.flow
                {
                    if let Some(signaling_to_actr::Payload::RegisterResponse(response)) =
                        server_to_actr.payload
                    {
                        return Ok(response);
                    }
                }
            } else {
                return Err(NetworkError::ConnectionError(
                    "Connection closed while waiting for registration response".to_string(),
                ));
            }
        }
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
            sticky_client_ids: vec![],  // TODO: Implement sticky session tracking
        };

        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::Ping(ping)),
        });

        let envelope = self.create_envelope(flow).await;
        self.send_envelope(envelope).await
    }

    async fn send_envelope(&self, envelope: SignalingEnvelope) -> NetworkResult<()> {
        let mut sink_guard = self.ws_sink.lock().await;

        if let Some(sink) = sink_guard.as_mut() {
            // using protobuf binary serialization
            let mut buf = Vec::new();
            envelope.encode(&mut buf)?;
            let msg = tokio_tungstenite::tungstenite::Message::Binary(buf.into());
            sink.send(msg).await?;

            let mut stats = self.stats.write().await;
            stats.messages_sent += 1;

            Ok(())
        } else {
            Err(NetworkError::ConnectionError("Not connected".to_string()))
        }
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        let mut stream_guard = self.ws_stream.lock().await;

        if let Some(stream) = stream_guard.as_mut() {
            if let Some(msg) = stream.next().await {
                let msg = msg?;
                match msg {
                    tokio_tungstenite::tungstenite::Message::Binary(data) => {
                        // using protobuf decode
                        let envelope = SignalingEnvelope::decode(&data[..])?;

                        let mut stats = self.stats.write().await;
                        stats.messages_received += 1;

                        Ok(Some(envelope))
                    }
                    _ => Ok(None),
                }
            } else {
                Ok(None)
            }
        } else {
            Err(NetworkError::ConnectionError("Not connected".to_string()))
        }
    }

    fn is_connected(&self) -> bool {
        // using blocking API read connection status
        // Note： this inmayinasynccontext in adjust usage ， but RwLock::blocking_read() in short temporal duration hold has temporal is safe secure 's
        *self.connected.blocking_read()
    }

    fn get_stats(&self) -> SignalingStats {
        // using blocking API read statistics info
        self.stats.blocking_read().clone()
    }
}

/// signaling statistics info
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
