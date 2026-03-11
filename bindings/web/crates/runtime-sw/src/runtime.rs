//! Client Runtime for Service Worker
//!
//! 按照文档架构实现的 Service Worker 客户端运行时：
//! - State Path：Mailbox + MailboxProcessor（可靠消息处理）
//! - Fast Path：Stream/Media 直接回调（低延迟）
//! - WebRTC signaling relay（通过 DOM coordinator）
//!
//! # 消息流
//!
//! ## 发送请求（DOM → Remote）
//! ```text
//! DOM → handle_dom_control → SERVICE_HANDLER (UnifiedDispatcher)
//!     → local route: handler(route_key, payload, ctx) → response
//!     → remote route: ctx.call_raw() → Gate → WebRTC DataChannel
//! ```
//!
//! ## 接收响应/请求（Remote → SW）
//! ```text
//! WebRTC → handle_fast_path → InboundPacketDispatcher.dispatch()
//!   → RPC 响应: 直接匹配 pending request → DOM
//!   → RPC 请求: Mailbox → MailboxProcessor → Actor 处理
//! ```

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use actr_mailbox_web::{IndexedDbMailbox, Mailbox, MessageRecord};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, Acl, AclRule, ActrId, ActrIdExt, ActrToSignaling, ActrType, ActrTypeExt,
    PeerToSignaling, Ping, RegisterRequest, RoleNegotiation, RouteCandidatesRequest, RpcEnvelope,
    ServiceAvailabilityState, SignalingEnvelope, acl_rule, actr_relay, actr_to_signaling,
    peer_to_signaling, route_candidates_request, session_description, signaling_envelope,
    signaling_to_actr,
};
use actr_protocol::{IceCandidate, SessionDescription, prost_types};
use actr_web_common::{ExponentialBackoff, MessageFormat, PayloadType};
use bytes::Bytes;
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::lock::Mutex;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Object, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{BinaryType, CloseEvent, MessageEvent, MessagePort, WebSocket};

use crate::context::RuntimeContext;
use crate::inbound::{InboundPacketDispatcher, MailboxProcessor, Scheduler};
use crate::outbound::Gate;
use crate::web_context::RuntimeBridge;

type StreamHandler = Rc<RefCell<Box<dyn FnMut(Bytes)>>>;

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

#[derive(Serialize)]
struct RpcResponsePayload {
    request_id: String,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    data: JsValue,
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct WebRtcCommandPayload {
    action: String,
    #[serde(rename = "peerId")]
    peer_id: String,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    payload: JsValue,
}

#[derive(Serialize)]
struct SdpPayload {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    sdp: JsValue,
}

#[derive(Serialize)]
struct IcePayload {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    candidate: JsValue,
}

#[derive(Serialize)]
struct SendDataPayload {
    #[serde(rename = "channelId")]
    channel_id: u32,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    data: JsValue,
}

#[derive(Serialize)]
struct SwMessage<T> {
    #[serde(rename = "type")]
    msg_type: &'static str,
    payload: T,
}

#[derive(Clone)]
struct SignalingClient {
    ws: WebSocket,
    pending: Rc<Mutex<HashMap<String, oneshot::Sender<SignalingEnvelope>>>>,
    /// Sender side of inbound channel; stored so we can close it on disconnect.
    inbound_tx: Rc<RefCell<Option<mpsc::UnboundedSender<SignalingEnvelope>>>>,
    inbound_rx: Rc<Mutex<mpsc::UnboundedReceiver<SignalingEnvelope>>>,
    envelope_counter: Rc<std::cell::Cell<u64>>,
    _onmessage: Rc<Closure<dyn FnMut(MessageEvent)>>,
    _onclose: Rc<Closure<dyn FnMut(CloseEvent)>>,
}

impl SignalingClient {
    async fn connect(url: &str) -> Result<Self, JsValue> {
        let ws = WebSocket::new(url)?;
        ws.set_binary_type(BinaryType::Arraybuffer);

        let pending: Rc<Mutex<HashMap<String, oneshot::Sender<SignalingEnvelope>>>> =
            Rc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::unbounded();
        let inbound_rx = Rc::new(Mutex::new(rx));
        let envelope_counter = Rc::new(std::cell::Cell::new(0));

        let pending_clone = Rc::clone(&pending);
        let tx_clone = tx.clone();
        let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
            let data = match event.data().dyn_into::<js_sys::ArrayBuffer>() {
                Ok(buf) => {
                    let array = js_sys::Uint8Array::new(&buf);
                    let mut bytes = vec![0u8; array.length() as usize];
                    array.copy_to(&mut bytes);
                    bytes
                }
                Err(_) => return,
            };

            let envelope = match SignalingEnvelope::decode(&data[..]) {
                Ok(env) => env,
                Err(e) => {
                    log::error!("[Signaling] Failed to decode envelope: {:?}", e);
                    return;
                }
            };

            let pending_clone = Rc::clone(&pending_clone);
            let tx_clone = tx_clone.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(reply_for) = envelope.reply_for.clone() {
                    if let Some(tx) = pending_clone.lock().await.remove(&reply_for) {
                        let _ = tx.send(envelope);
                        return;
                    }
                }
                let _ = tx_clone.unbounded_send(envelope);
            });
        }) as Box<dyn FnMut(MessageEvent)>);

        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

        // Close the inbound channel when the WebSocket closes so that
        // recv_inbound() returns None and the relay loop terminates.
        let inbound_tx: Rc<RefCell<Option<mpsc::UnboundedSender<SignalingEnvelope>>>> =
            Rc::new(RefCell::new(Some(tx)));
        let inbound_tx_for_close = Rc::clone(&inbound_tx);
        let onclose = Closure::wrap(Box::new(move |_event: CloseEvent| {
            log::info!("[Signaling] WebSocket closed, closing inbound channel");
            if let Some(tx) = inbound_tx_for_close.borrow_mut().take() {
                tx.close_channel();
            }
        }) as Box<dyn FnMut(CloseEvent)>);
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));

        // Wait for open (max 15s)
        let ws_clone = ws.clone();
        let open_future = async move {
            let start = js_sys::Date::now();
            loop {
                if ws_clone.ready_state() == WebSocket::OPEN {
                    return Ok(());
                }
                if ws_clone.ready_state() == WebSocket::CLOSED {
                    return Err(JsValue::from_str("WebSocket closed"));
                }
                if js_sys::Date::now() - start > 15000.0 {
                    return Err(JsValue::from_str("WebSocket connect timeout"));
                }
                TimeoutFuture::new(10).await;
            }
        };
        open_future.await?;

        Ok(Self {
            ws,
            pending,
            inbound_tx,
            inbound_rx,
            envelope_counter,
            _onmessage: Rc::new(onmessage),
            _onclose: Rc::new(onclose),
        })
    }

    /// Connect to signaling server with retry and exponential backoff.
    ///
    /// Mirrors the `connect_with_retries` logic from the native `actr` runtime:
    /// - If `reconnect_enabled` is false, attempts a single connection.
    /// - Otherwise, retries up to `max_attempts` times with exponential backoff.
    async fn connect_with_retries(
        url: &str,
        reconnect_cfg: &ReconnectConfig,
    ) -> Result<Self, JsValue> {
        if !reconnect_cfg.enabled {
            return Self::connect(url).await;
        }

        let mut backoff =
            ExponentialBackoff::new(reconnect_cfg.initial_delay_ms, reconnect_cfg.max_delay_ms)
                .with_multiplier(reconnect_cfg.backoff_multiplier)
                .with_jitter(0.1);

        let mut attempt: u32 = 0;

        loop {
            attempt += 1;

            match Self::connect(url).await {
                Ok(client) => {
                    if attempt > 1 {
                        log::info!("[Signaling] Connected after {} attempts", attempt);
                    }
                    return Ok(client);
                }
                Err(e) => {
                    log::warn!(
                        "[Signaling] Connect attempt {}/{} failed: {:?}",
                        attempt,
                        reconnect_cfg.max_attempts,
                        e
                    );

                    if attempt >= reconnect_cfg.max_attempts {
                        log::error!(
                            "[Signaling] Connect failed after {} attempts, giving up",
                            attempt
                        );
                        return Err(e);
                    }

                    let delay = backoff.next_delay();
                    let delay_ms = delay.as_millis() as u32;
                    log::info!("[Signaling] Retry connect after {}ms", delay_ms);
                    TimeoutFuture::new(delay_ms).await;
                }
            }
        }
    }

    fn next_envelope_id(&self) -> String {
        let next = self.envelope_counter.get() + 1;
        self.envelope_counter.set(next);
        format!("sw-env-{}-{}", next, js_sys::Date::now() as u64)
    }

    fn now_timestamp() -> prost_types::Timestamp {
        let ms = js_sys::Date::now() as i64;
        prost_types::Timestamp {
            seconds: ms / 1000,
            nanos: ((ms % 1000) * 1_000_000) as i32,
        }
    }

    async fn send_envelope(&self, mut envelope: SignalingEnvelope) -> Result<(), JsValue> {
        if envelope.envelope_id.is_empty() {
            envelope.envelope_id = self.next_envelope_id();
        }
        if envelope.envelope_version == 0 {
            envelope.envelope_version = 1;
        }
        envelope.timestamp = Self::now_timestamp();

        let bytes = envelope.encode_to_vec();
        self.ws.send_with_u8_array(&bytes)?;
        Ok(())
    }

    async fn send_request(
        &self,
        envelope: SignalingEnvelope,
    ) -> Result<SignalingEnvelope, JsValue> {
        let mut envelope = envelope;
        if envelope.envelope_id.is_empty() {
            envelope.envelope_id = self.next_envelope_id();
        }
        let reply_for = envelope.envelope_id.clone();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(reply_for, tx);
        self.send_envelope(envelope).await?;

        rx.await
            .map_err(|_| JsValue::from_str("signaling reply channel closed"))
    }

    async fn recv_inbound(&self) -> Option<SignalingEnvelope> {
        self.inbound_rx.lock().await.next().await
    }

    /// Close the underlying WebSocket connection.
    ///
    /// This triggers cleanup on the signaling server side (which removes
    /// the actor from the ServiceRegistry) and causes background loops
    /// (heartbeat, relay) to naturally terminate.
    fn close(&self) {
        let _ = self.ws.close();
        // Also close the inbound channel immediately so recv_inbound()
        // returns None without waiting for the async onclose event.
        if let Some(tx) = self.inbound_tx.borrow_mut().take() {
            tx.close_channel();
        }
    }
}

/// Reconnection configuration for the initial signaling connection.
///
/// Mirrors `ReconnectConfig` from the native `actr` runtime.
#[derive(Debug, Clone, Deserialize)]
struct ReconnectConfig {
    /// Whether automatic reconnection is enabled.
    #[serde(default = "ReconnectConfig::default_enabled")]
    enabled: bool,

    /// Maximum number of connection attempts.
    #[serde(default = "ReconnectConfig::default_max_attempts")]
    max_attempts: u32,

    /// Initial retry delay in milliseconds.
    #[serde(default = "ReconnectConfig::default_initial_delay_ms")]
    initial_delay_ms: u64,

    /// Maximum retry delay in milliseconds.
    #[serde(default = "ReconnectConfig::default_max_delay_ms")]
    max_delay_ms: u64,

    /// Backoff multiplier factor.
    #[serde(default = "ReconnectConfig::default_backoff_multiplier")]
    backoff_multiplier: f64,
}

impl ReconnectConfig {
    fn default_enabled() -> bool {
        true
    }
    fn default_max_attempts() -> u32 {
        10
    }
    fn default_initial_delay_ms() -> u64 {
        1000
    }
    fn default_max_delay_ms() -> u64 {
        60000
    }
    fn default_backoff_multiplier() -> f64 {
        2.0
    }
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 10,
            initial_delay_ms: 1000, // 1s
            max_delay_ms: 60000,    // 60s
            backoff_multiplier: 2.0,
        }
    }
}

#[derive(Deserialize)]
struct SwConfig {
    signaling_url: String,
    realm_id: u32,
    client_actr_type: String,
    target_actr_type: String,
    service_fingerprint: String,
    /// ACL: list of actr_types (e.g. "acme:echo-client-app") that are allowed
    /// to discover and communicate with this actor. Sent in RegisterRequest.
    #[serde(default)]
    acl_allow_types: Vec<String>,
    /// If true, this actor is a server and will NOT call discover_target
    /// after registration. It passively waits for incoming connections.
    #[serde(default)]
    is_server: bool,
    /// Reconnection configuration for the initial signaling WebSocket connection.
    /// If omitted, defaults are used (enabled, 10 attempts, 1s–60s exponential backoff).
    #[serde(default)]
    reconnect_config: ReconnectConfig,
}

#[derive(Deserialize)]
struct DomRpcCall {
    action: String,
    request_id: String,
    request: DomRpcRequest,
}

#[derive(Deserialize)]
struct DomRpcRequest {
    route_key: String,
    payload: Vec<u8>,
    timeout: Option<u32>,
}

#[derive(Deserialize)]
struct DomWebRtcEvent {
    #[serde(rename = "eventType")]
    event_type: String,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct LocalDescriptionEvent {
    #[serde(rename = "peerId")]
    peer_id: String,
    sdp: SdpInit,
}

#[derive(Deserialize)]
struct SdpInit {
    #[serde(rename = "type")]
    sdp_type: String,
    sdp: String,
}

#[derive(Deserialize)]
struct IceCandidateEvent {
    #[serde(rename = "peerId")]
    peer_id: String,
    candidate: IceCandidateInit,
}

#[derive(Deserialize)]
struct IceCandidateInit {
    candidate: String,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u32>,
    #[serde(rename = "usernameFragment")]
    username_fragment: Option<String>,
}

#[derive(Deserialize)]
struct DataChannelEvent {
    #[serde(rename = "peerId")]
    peer_id: String,
    #[serde(rename = "channelId")]
    channel_id: u32,
}

#[derive(Deserialize)]
struct FastPathPayload {
    #[serde(rename = "streamId")]
    stream_id: String,
    data: Vec<u8>,
}

/// Distinguishes between DOM-originated and handler-internal pending RPCs.
///
/// - `Dom`: response must be sent back to the DOM as a `control_response` message.
/// - `Internal`: response is consumed by the HostGate oneshot only (handler-initiated remote calls).
#[derive(Clone, Debug)]
pub enum PendingRpcTarget {
    Dom,
    Internal,
}

/// ICE restart configuration constants
const ICE_RESTART_MAX_RETRIES: u32 = 2;
const ICE_RESTART_TIMEOUT_MS: u32 = 3000;
const ICE_RESTART_INITIAL_BACKOFF_MS: u32 = 2000;
const ICE_RESTART_MAX_BACKOFF_MS: u32 = 5000;

/// P2P initial connection retry configuration.
///
/// Mirrors the retry strategy from the native `actr` runtime's
/// `create_connection_inner` (3 retries, 5s→15s backoff, 30s overall).
const P2P_CONNECTION_MAX_RETRIES: u32 = 3;
const P2P_RETRY_INITIAL_DELAY_MS: u64 = 3000;
const P2P_RETRY_MAX_DELAY_MS: u64 = 15000;

struct SwRuntime {
    /// Unique client identifier (one per browser tab)
    client_id: String,
    signaling_url: String,
    realm_id: u32,
    client_actr_type: ActrType,
    target_actr_type: ActrType,
    service_fingerprint: String,
    acl: Option<Acl>,
    is_server: bool,
    signaling: SignalingClient,
    actor_id: Option<ActrId>,
    credential: Option<AIdCredential>,
    target_id: Option<ActrId>,
    dom_port: Option<MessagePort>,
    pending_rpcs: HashMap<String, PendingRpcTarget>,
    known_peers: HashSet<String>,
    open_channels: HashMap<String, HashSet<u32>>,
    pending_channel_data: HashMap<String, Vec<Vec<u8>>>,
    role_negotiated: HashSet<String>,
    role_assignments: HashMap<String, bool>,
    /// ICE restart: tracks whether an ICE restart is in-flight for each peer
    ice_restart_inflight: HashMap<String, bool>,
    /// ICE restart: retry attempt count per peer
    ice_restart_attempts: HashMap<String, u32>,
    /// ICE restart: tracks the connection state for each peer
    peer_connection_states: HashMap<String, String>,
}

impl SwRuntime {
    async fn new(client_id: String, config: SwConfig) -> Result<Self, JsValue> {
        let signaling =
            SignalingClient::connect_with_retries(&config.signaling_url, &config.reconnect_config)
                .await?;

        // Build ACL from config
        let acl = if config.acl_allow_types.is_empty() {
            None
        } else {
            let principals: Vec<acl_rule::Principal> = config
                .acl_allow_types
                .iter()
                .map(|type_str| {
                    let actr_type = ActrType::from_string_repr(type_str).map_err(|e| {
                        JsValue::from_str(&format!("Invalid ACL type '{}': {}", type_str, e))
                    })?;
                    Ok(acl_rule::Principal {
                        realm: None,
                        actr_type: Some(actr_type),
                    })
                })
                .collect::<Result<Vec<_>, JsValue>>()?;
            Some(Acl {
                rules: vec![AclRule {
                    principals,
                    permission: acl_rule::Permission::Allow as i32,
                }],
            })
        };

        Ok(Self {
            client_id,
            signaling_url: config.signaling_url,
            realm_id: config.realm_id,
            client_actr_type: ActrType::from_string_repr(&config.client_actr_type)
                .map_err(|e| JsValue::from_str(&format!("Invalid client actr_type: {e}")))?,
            target_actr_type: ActrType::from_string_repr(&config.target_actr_type)
                .map_err(|e| JsValue::from_str(&format!("Invalid target actr_type: {e}")))?,
            service_fingerprint: config.service_fingerprint,
            acl,
            is_server: config.is_server,
            signaling,
            actor_id: None,
            credential: None,
            target_id: None,
            dom_port: None,
            pending_rpcs: HashMap::new(),
            known_peers: HashSet::new(),
            open_channels: HashMap::new(),
            pending_channel_data: HashMap::new(),
            role_negotiated: HashSet::new(),
            role_assignments: HashMap::new(),
            ice_restart_inflight: HashMap::new(),
            ice_restart_attempts: HashMap::new(),
            peer_connection_states: HashMap::new(),
        })
    }

    async fn register(&mut self) -> Result<(), JsValue> {
        log::info!(
            "[SW] register: sending RegisterRequest (acl={:?})",
            self.acl.is_some()
        );
        let request = RegisterRequest {
            actr_type: self.client_actr_type.clone(),
            realm: actr_protocol::Realm {
                realm_id: self.realm_id,
            },
            service_spec: None,
            acl: self.acl.clone(),
            service: None,
            ws_address: None,
        };
        let envelope = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: self.signaling.next_envelope_id(),
            reply_for: None,
            timestamp: SignalingClient::now_timestamp(),
            traceparent: None,
            tracestate: None,
            flow: Some(signaling_envelope::Flow::PeerToServer(PeerToSignaling {
                payload: Some(peer_to_signaling::Payload::RegisterRequest(request)),
            })),
        };

        let response = self.signaling.send_request(envelope).await?;
        log::info!("[SW] register: got response");
        let register_response = match response.flow {
            Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) => {
                match server_to_actr.payload {
                    Some(signaling_to_actr::Payload::RegisterResponse(resp)) => resp,
                    _ => return Err(JsValue::from_str("Unexpected signaling response payload")),
                }
            }
            _ => return Err(JsValue::from_str("Unexpected signaling response flow")),
        };

        match register_response.result {
            Some(actr_protocol::register_response::Result::Success(ok)) => {
                log::info!(
                    "[SW] register: success actr_id={}",
                    ok.actr_id.to_string_repr()
                );
                self.actor_id = Some(ok.actr_id);
                self.credential = Some(ok.credential);
                Ok(())
            }
            Some(actr_protocol::register_response::Result::Error(err)) => {
                log::warn!("[SW] register: error {}", err.message);
                Err(JsValue::from_str(&format!(
                    "Register failed: {}",
                    err.message
                )))
            }
            None => Err(JsValue::from_str("Register response missing result")),
        }
    }

    /// Reconnect the signaling WebSocket and re-register with the
    /// signaling server.  Called when the heartbeat detects that the
    /// existing WebSocket is dead (e.g. browser killed the SW, network
    /// interruption, server restart).
    ///
    /// After a successful reconnect the caller must spawn a new signaling
    /// relay loop because the old one terminated when the old channel
    /// was closed.
    async fn reconnect_signaling(&mut self) -> Result<(), JsValue> {
        log::info!(
            "[SW] [{}] Reconnecting signaling WebSocket...",
            self.client_id
        );

        // 1. Close old signaling (idempotent — may already be closed).
        self.signaling.close();

        // 2. Clear stale peer / target state so the next RPC forces
        //    a fresh discovery & WebRTC handshake.
        self.target_id = None;
        self.known_peers.clear();
        self.open_channels.clear();
        self.pending_channel_data.clear();
        self.role_negotiated.clear();
        self.role_assignments.clear();
        self.ice_restart_inflight.clear();
        self.ice_restart_attempts.clear();
        self.peer_connection_states.clear();

        // 3. Create a brand-new signaling client (new WebSocket).
        let reconnect_config = ReconnectConfig::default();
        self.signaling =
            SignalingClient::connect_with_retries(&self.signaling_url, &reconnect_config).await?;

        // 4. Re-register with the signaling server to obtain a fresh
        //    actr_id + credential and restore the service entry.
        self.register().await?;

        log::info!(
            "[SW] [{}] Signaling reconnected and re-registered",
            self.client_id
        );
        Ok(())
    }

    /// Send a signaling heartbeat (Ping) to keep the connection alive
    /// and the service registration active.
    async fn send_heartbeat(&self) -> Result<(), JsValue> {
        let actor_id = self
            .actor_id
            .clone()
            .ok_or_else(|| JsValue::from_str("Actor not registered"))?;
        let credential = self
            .credential
            .clone()
            .ok_or_else(|| JsValue::from_str("Missing credential"))?;

        let ping = Ping {
            availability: ServiceAvailabilityState::Full as i32,
            power_reserve: 0.01,
            mailbox_backlog: 0.0,
            sticky_client_ids: vec![],
        };
        let envelope = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: self.signaling.next_envelope_id(),
            reply_for: None,
            timestamp: SignalingClient::now_timestamp(),
            traceparent: None,
            tracestate: None,
            flow: Some(signaling_envelope::Flow::ActrToServer(ActrToSignaling {
                source: actor_id,
                credential,
                payload: Some(actr_to_signaling::Payload::Ping(ping)),
            })),
        };
        self.signaling.send_request(envelope).await?;
        log::debug!("[SW] heartbeat sent");
        Ok(())
    }

    async fn discover_target(&mut self) -> Result<ActrId, JsValue> {
        if self.is_server {
            log::info!(
                "[SW] discover_target: skipped (server mode, waiting for incoming connections)"
            );
            return Err(JsValue::from_str("Server mode: no target discovery needed"));
        }
        if let Some(target) = self.target_id.clone() {
            return Ok(target);
        }
        let actor_id = self
            .actor_id
            .clone()
            .ok_or_else(|| JsValue::from_str("Actor not registered"))?;
        let credential = self
            .credential
            .clone()
            .ok_or_else(|| JsValue::from_str("Missing credential"))?;

        log::info!(
            "[SW] route_candidates: request target_type={}",
            self.target_actr_type.to_string_repr()
        );
        let criteria = route_candidates_request::NodeSelectionCriteria {
            candidate_count: 1,
            ranking_factors: vec![route_candidates_request::node_selection_criteria::NodeRankingFactor::MaximumPowerReserve as i32],
            minimal_dependency_requirement: None,
            minimal_health_requirement: None,
        };
        let request = RouteCandidatesRequest {
            target_type: self.target_actr_type.clone(),
            criteria: Some(criteria),
            client_location: None,
            client_fingerprint: self.service_fingerprint.clone(),
        };

        let envelope = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: self.signaling.next_envelope_id(),
            reply_for: None,
            timestamp: SignalingClient::now_timestamp(),
            traceparent: None,
            tracestate: None,
            flow: Some(signaling_envelope::Flow::ActrToServer(ActrToSignaling {
                source: actor_id,
                credential,
                payload: Some(
                    actr_protocol::actr_to_signaling::Payload::RouteCandidatesRequest(request),
                ),
            })),
        };

        let response = self.signaling.send_request(envelope).await?;
        log::info!("[SW] route_candidates: got response");
        let route_response = match response.flow {
            Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) => {
                match server_to_actr.payload {
                    Some(signaling_to_actr::Payload::RouteCandidatesResponse(resp)) => resp,
                    _ => {
                        return Err(JsValue::from_str(
                            "Unexpected signaling route response payload",
                        ));
                    }
                }
            }
            _ => {
                return Err(JsValue::from_str(
                    "Unexpected signaling route response flow",
                ));
            }
        };

        match route_response.result {
            Some(actr_protocol::route_candidates_response::Result::Success(ok)) => {
                let target = ok
                    .candidates
                    .first()
                    .cloned()
                    .ok_or_else(|| JsValue::from_str("No candidates"))?;
                log::info!(
                    "[SW] route_candidates: selected {}",
                    target.to_string_repr()
                );
                self.target_id = Some(target.clone());
                Ok(target)
            }
            Some(actr_protocol::route_candidates_response::Result::Error(err)) => {
                log::warn!("[SW] route_candidates: error {}", err.message);
                Err(JsValue::from_str(&format!(
                    "Route candidates error: {}",
                    err.message
                )))
            }
            None => Err(JsValue::from_str("Route candidates missing result")),
        }
    }

    /// 使用指定的 ActrType 发现目标 Actor
    ///
    /// 与 `discover_target` 类似，但允许指定目标类型而非使用配置中的默认类型
    async fn discover_target_for_type(
        &mut self,
        target_type: &ActrType,
    ) -> Result<ActrId, JsValue> {
        let actor_id = self
            .actor_id
            .clone()
            .ok_or_else(|| JsValue::from_str("Actor not registered"))?;
        let credential = self
            .credential
            .clone()
            .ok_or_else(|| JsValue::from_str("Missing credential"))?;

        log::info!(
            "[SW] discover_target_for_type: request target_type={}",
            target_type.to_string_repr()
        );
        let criteria = route_candidates_request::NodeSelectionCriteria {
            candidate_count: 1,
            ranking_factors: vec![route_candidates_request::node_selection_criteria::NodeRankingFactor::MaximumPowerReserve as i32],
            minimal_dependency_requirement: None,
            minimal_health_requirement: None,
        };
        let request = RouteCandidatesRequest {
            target_type: target_type.clone(),
            criteria: Some(criteria),
            client_location: None,
            client_fingerprint: self.service_fingerprint.clone(),
        };

        let envelope = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: self.signaling.next_envelope_id(),
            reply_for: None,
            timestamp: SignalingClient::now_timestamp(),
            traceparent: None,
            tracestate: None,
            flow: Some(signaling_envelope::Flow::ActrToServer(ActrToSignaling {
                source: actor_id,
                credential,
                payload: Some(
                    actr_protocol::actr_to_signaling::Payload::RouteCandidatesRequest(request),
                ),
            })),
        };

        let response = self.signaling.send_request(envelope).await?;
        let route_response = match response.flow {
            Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) => {
                match server_to_actr.payload {
                    Some(signaling_to_actr::Payload::RouteCandidatesResponse(resp)) => resp,
                    _ => {
                        return Err(JsValue::from_str(
                            "Unexpected signaling route response payload",
                        ));
                    }
                }
            }
            _ => {
                return Err(JsValue::from_str(
                    "Unexpected signaling route response flow",
                ));
            }
        };

        match route_response.result {
            Some(actr_protocol::route_candidates_response::Result::Success(ok)) => {
                let target = ok
                    .candidates
                    .first()
                    .cloned()
                    .ok_or_else(|| JsValue::from_str("No candidates"))?;
                log::info!(
                    "[SW] discover_target_for_type: selected {}",
                    target.to_string_repr()
                );
                Ok(target)
            }
            Some(actr_protocol::route_candidates_response::Result::Error(err)) => Err(
                JsValue::from_str(&format!("Route candidates error: {}", err.message)),
            ),
            None => Err(JsValue::from_str("Route candidates missing result")),
        }
    }

    fn send_dom_message(&self, msg: &JsValue) -> Result<(), JsValue> {
        let port = self
            .dom_port
            .as_ref()
            .ok_or_else(|| JsValue::from_str("DOM port not set"))?;
        port.post_message(msg)?;
        Ok(())
    }

    fn send_webrtc_command(
        &self,
        action: &str,
        peer_id: &str,
        payload: JsValue,
    ) -> Result<(), JsValue> {
        log::info!("[SW] webrtc_command: {} peer={}", action, peer_id);

        let response = SwMessage {
            msg_type: "webrtc_command",
            payload: WebRtcCommandPayload {
                action: action.to_string(),
                peer_id: peer_id.to_string(),
                payload,
            },
        };

        let msg_js_value = serde_wasm_bindgen::to_value(&response).map_err(|e| {
            JsValue::from_str(&format!("Failed to serialize WebRTC command: {}", e))
        })?;

        self.send_dom_message(&msg_js_value)
    }

    async fn send_role_negotiation(&self, target: &ActrId) -> Result<(), JsValue> {
        let actor_id = self
            .actor_id
            .clone()
            .ok_or_else(|| JsValue::from_str("Actor not registered"))?;
        let credential = self
            .credential
            .clone()
            .ok_or_else(|| JsValue::from_str("Missing credential"))?;
        log::info!(
            "[SW] role_negotiation: from={} to={}",
            actor_id.to_string_repr(),
            target.to_string_repr()
        );
        let payload = actr_relay::Payload::RoleNegotiation(RoleNegotiation {
            from: actor_id.clone(),
            to: target.clone(),
            realm_id: actor_id.realm.realm_id,
        });
        let relay = actr_protocol::ActrRelay {
            source: actor_id,
            credential,
            target: target.clone(),
            payload: Some(payload),
        };
        let envelope = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: self.signaling.next_envelope_id(),
            reply_for: None,
            timestamp: SignalingClient::now_timestamp(),
            traceparent: None,
            tracestate: None,
            flow: Some(signaling_envelope::Flow::ActrRelay(relay)),
        };
        self.signaling.send_envelope(envelope).await
    }

    async fn ensure_peer(&mut self) -> Result<String, JsValue> {
        let target = self.discover_target().await?;
        let peer_id = target.to_string_repr();
        if !self.known_peers.contains(&peer_id) {
            self.send_webrtc_command("create_peer", &peer_id, JsValue::NULL)?;
            self.known_peers.insert(peer_id.clone());
        }
        if !self.role_negotiated.contains(&peer_id) {
            self.send_role_negotiation(&target).await?;
            self.role_negotiated.insert(peer_id.clone());
        }
        Ok(peer_id)
    }

    /// Ensure a peer connection is initiated, with retry and exponential backoff.
    ///
    /// Mirrors the `create_connection_inner` retry logic from the native `actr`
    /// runtime: retries `ensure_peer()` up to `P2P_CONNECTION_MAX_RETRIES` times
    /// with exponential backoff. This handles transient failures such as:
    /// - Target actor not yet registered ("No candidates")
    /// - Signaling network errors during role negotiation
    async fn ensure_peer_with_retry(&mut self) -> Result<String, JsValue> {
        let mut backoff =
            ExponentialBackoff::new(P2P_RETRY_INITIAL_DELAY_MS, P2P_RETRY_MAX_DELAY_MS)
                .with_multiplier(2.0)
                .with_jitter(0.1);

        let mut attempt: u32 = 0;

        loop {
            attempt += 1;

            match self.ensure_peer().await {
                Ok(peer_id) => {
                    if attempt > 1 {
                        log::info!("[SW] P2P peer established after {} attempts", attempt);
                    }
                    return Ok(peer_id);
                }
                Err(e) => {
                    log::warn!(
                        "[SW] P2P connection attempt {}/{} failed: {:?}",
                        attempt,
                        P2P_CONNECTION_MAX_RETRIES,
                        e
                    );

                    if attempt >= P2P_CONNECTION_MAX_RETRIES {
                        log::error!(
                            "[SW] P2P connection failed after {} attempts, giving up",
                            attempt
                        );
                        return Err(e);
                    }

                    let delay = backoff.next_delay();
                    let delay_ms = delay.as_millis() as u32;
                    log::info!("[SW] Retry P2P connection after {}ms", delay_ms);
                    TimeoutFuture::new(delay_ms).await;
                }
            }
        }
    }

    /// Discover the target actor with retry and exponential backoff.
    ///
    /// This is a standalone retry wrapper for `discover_target()`, used by
    /// callers that need discovery but not the full `ensure_peer()` flow.
    /// Handles the common case where the server actor hasn't registered yet.
    async fn discover_target_with_retry(&mut self) -> Result<ActrId, JsValue> {
        let mut backoff =
            ExponentialBackoff::new(P2P_RETRY_INITIAL_DELAY_MS, P2P_RETRY_MAX_DELAY_MS)
                .with_multiplier(2.0)
                .with_jitter(0.1);

        let mut attempt: u32 = 0;

        loop {
            attempt += 1;

            match self.discover_target().await {
                Ok(target) => {
                    if attempt > 1 {
                        log::info!("[SW] Target discovered after {} attempts", attempt);
                    }
                    return Ok(target);
                }
                Err(e) => {
                    log::warn!(
                        "[SW] Target discovery attempt {}/{} failed: {:?}",
                        attempt,
                        P2P_CONNECTION_MAX_RETRIES,
                        e
                    );

                    if attempt >= P2P_CONNECTION_MAX_RETRIES {
                        log::error!(
                            "[SW] Target discovery failed after {} attempts, giving up",
                            attempt
                        );
                        return Err(e);
                    }

                    let delay = backoff.next_delay();
                    let delay_ms = delay.as_millis() as u32;
                    log::info!("[SW] Retry target discovery after {}ms", delay_ms);
                    TimeoutFuture::new(delay_ms).await;
                }
            }
        }
    }

    /// Register a pending RPC (callable from RuntimeContext for internal calls)
    fn register_pending_rpc(&mut self, request_id: String, target: PendingRpcTarget) {
        self.pending_rpcs.insert(request_id, target);
    }

    /// 处理来自远程的 RPC 响应
    ///
    /// 按照文档 6.1 消息流：
    /// 远程响应 → handle_fast_path → handle_rpc_response → System.handle_remote_response
    /// → HostGate.handle_response → DOM 收到响应
    fn handle_rpc_response(&mut self, envelope: RpcEnvelope) -> Result<(), JsValue> {
        let request_id = envelope.request_id.clone();

        // 检查是否是我们发送的请求
        let Some(rpc_target) = self.pending_rpcs.remove(&request_id) else {
            log::warn!(
                "[SW] Received response for unknown request_id: {}",
                request_id
            );
            return Ok(());
        };

        log::info!(
            "[SW] handle_rpc_response: request_id={} target={:?}",
            request_id,
            rpc_target
        );

        // 提取 payload 用于后续处理
        let payload_bytes = envelope
            .payload
            .as_ref()
            .map(|p| p.to_vec())
            .unwrap_or_default();

        // 通过 System 处理响应
        // 这将触发 HostGate.handle_response()
        CLIENTS.with(|cell| {
            if let Some(ctx) = cell.borrow().get(&self.client_id) {
                ctx.system
                    .handle_remote_response(&request_id, Bytes::from(payload_bytes.clone()));
            } else {
                log::warn!(
                    "[SW] Client context not found for client_id={}",
                    self.client_id
                );
            }
        });

        // Only send to DOM for DOM-originated requests
        match rpc_target {
            PendingRpcTarget::Dom => {
                let js_payload: JsValue = if payload_bytes.is_empty() {
                    JsValue::NULL
                } else {
                    js_sys::Uint8Array::from(payload_bytes.as_slice()).into()
                };

                let response = SwMessage {
                    msg_type: "control_response",
                    payload: RpcResponsePayload {
                        request_id,
                        data: js_payload,
                        error: envelope.error.map(|e| RpcError {
                            code: e.code as i32,
                            message: e.message,
                        }),
                    },
                };

                let msg_js_value = serde_wasm_bindgen::to_value(&response).map_err(|e| {
                    JsValue::from_str(&format!("Failed to serialize RPC response: {}", e))
                })?;

                self.send_dom_message(&msg_js_value)?;
            }
            PendingRpcTarget::Internal => {
                // Internal (handler-initiated) RPCs: response handled via System/HostGate only
                log::debug!(
                    "[SW] Internal RPC response handled: request_id={}",
                    request_id
                );
            }
        }

        Ok(())
    }

    async fn handle_inbound_signaling(&mut self) -> Result<(), JsValue> {
        while let Some(env) = self.signaling.recv_inbound().await {
            if let Some(signaling_envelope::Flow::ActrRelay(relay)) = env.flow {
                self.handle_actr_relay(relay)?;
            }
        }
        Ok(())
    }

    fn handle_actr_relay(&mut self, relay: actr_protocol::ActrRelay) -> Result<(), JsValue> {
        let peer_id = relay.source.to_string_repr();
        match relay.payload {
            Some(actr_relay::Payload::SessionDescription(sd)) => {
                log::info!(
                    "[SW] relay: session_description from={} type={}",
                    peer_id,
                    sd.r#type
                );

                // Check if this is an ICE restart offer (type=3)
                let is_ice_restart = sd.r#type == session_description::Type::IceRestartOffer as i32;

                let sdp_type = match sd.r#type {
                    0 => "offer",  // OFFER
                    1 => "answer", // ANSWER
                    2 => "offer",  // RENEGOTIATION_OFFER (treated as offer)
                    3 => "offer",  // ICE_RESTART_OFFER (treated as offer for WebRTC API)
                    _ => "offer",
                };

                // Ensure peer exists before setting remote description
                if !self.known_peers.contains(&peer_id) {
                    self.send_webrtc_command("create_peer", &peer_id, JsValue::NULL)?;
                    self.known_peers.insert(peer_id.clone());
                }

                let sdp_obj = Object::new();
                Reflect::set(
                    &sdp_obj,
                    &JsValue::from_str("type"),
                    &JsValue::from_str(sdp_type),
                )?;
                Reflect::set(
                    &sdp_obj,
                    &JsValue::from_str("sdp"),
                    &JsValue::from_str(&sd.sdp),
                )?;

                let payload = Object::new();
                Reflect::set(&payload, &JsValue::from_str("sdp"), &sdp_obj)?;

                self.send_webrtc_command("set_remote_description", &peer_id, payload.into())?;

                if sdp_type == "offer" {
                    if is_ice_restart {
                        log::info!(
                            "[SW] ICE restart offer received from={}, creating answer",
                            peer_id
                        );
                    }
                    self.send_webrtc_command("create_answer", &peer_id, JsValue::NULL)?;
                }

                // If we received an answer during ICE restart, mark restart as complete
                if sd.r#type == session_description::Type::Answer as i32 {
                    if self.ice_restart_inflight.get(&peer_id) == Some(&true) {
                        log::info!(
                            "[SW] ICE restart: received answer from={}, waiting for datachannel reopen",
                            peer_id
                        );
                        // The restart will be fully complete when datachannel re-opens
                        // (handled in datachannel_open event)
                    }
                }
            }
            Some(actr_relay::Payload::IceCandidate(ice)) => {
                log::info!("[SW] relay: ice_candidate from={}", peer_id);
                let candidate = Object::new();
                Reflect::set(
                    &candidate,
                    &JsValue::from_str("candidate"),
                    &JsValue::from_str(&ice.candidate),
                )?;
                if let Some(mid) = ice.sdp_mid {
                    Reflect::set(
                        &candidate,
                        &JsValue::from_str("sdpMid"),
                        &JsValue::from_str(&mid),
                    )?;
                }
                if let Some(mline) = ice.sdp_mline_index {
                    Reflect::set(
                        &candidate,
                        &JsValue::from_str("sdpMLineIndex"),
                        &JsValue::from_f64(mline as f64),
                    )?;
                }
                if let Some(ufrag) = ice.username_fragment {
                    Reflect::set(
                        &candidate,
                        &JsValue::from_str("usernameFragment"),
                        &JsValue::from_str(&ufrag),
                    )?;
                }

                let payload = Object::new();
                Reflect::set(&payload, &JsValue::from_str("candidate"), &candidate)?;

                self.send_webrtc_command("add_ice_candidate", &peer_id, payload.into())?;
            }
            Some(actr_relay::Payload::RoleAssignment(assign)) => {
                // For RoleAssignment, relay.source may be our own ID (echoed back
                // by the signaling server). Determine the actual remote peer:
                // if source == self, use relay.target; otherwise use relay.source.
                let is_self = self
                    .actor_id
                    .as_ref()
                    .map(|id| id.to_string_repr() == peer_id)
                    .unwrap_or(false);
                let remote_peer_id = if is_self {
                    relay.target.to_string_repr()
                } else {
                    peer_id.clone()
                };

                log::info!(
                    "[SW] relay: role_assignment remote_peer={} is_offerer={}",
                    remote_peer_id,
                    assign.is_offerer
                );
                self.role_assignments
                    .insert(remote_peer_id.clone(), assign.is_offerer);
                if !self.known_peers.contains(&remote_peer_id) {
                    self.send_webrtc_command("create_peer", &remote_peer_id, JsValue::NULL)?;
                    self.known_peers.insert(remote_peer_id.clone());
                }
                if assign.is_offerer {
                    self.send_webrtc_command("create_offer", &remote_peer_id, JsValue::NULL)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_webrtc_event(&mut self, event: DomWebRtcEvent) -> Result<(), JsValue> {
        let actor_id = self
            .actor_id
            .clone()
            .ok_or_else(|| JsValue::from_str("Actor not registered"))?;
        let credential = self
            .credential
            .clone()
            .ok_or_else(|| JsValue::from_str("Missing credential"))?;

        log::info!(
            "[SW] webrtc_event: {} (is_server={})",
            event.event_type,
            self.is_server
        );

        // Determine target: for server mode, extract peerId from event data;
        // for client mode, use discover_target().
        let resolve_target = |event_data: &serde_json::Value| -> Result<ActrId, JsValue> {
            if let Some(peer_id_str) = event_data.get("peerId").and_then(|v| v.as_str()) {
                ActrId::from_string_repr(peer_id_str).map_err(|e| {
                    JsValue::from_str(&format!("Invalid peerId '{}': {}", peer_id_str, e))
                })
            } else {
                Err(JsValue::from_str("Missing peerId in event data"))
            }
        };

        match event.event_type.as_str() {
            "local_description" => {
                let data: LocalDescriptionEvent = serde_json::from_value(event.data.clone())
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;

                // Resolve target: server uses peerId from event, client uses discover_target
                let target = if self.is_server {
                    resolve_target(&event.data)?
                } else {
                    self.discover_target().await?
                };

                let sd_type = match data.sdp.sdp_type.as_str() {
                    "offer" => session_description::Type::Offer as i32,
                    "answer" => session_description::Type::Answer as i32,
                    _ => session_description::Type::Offer as i32,
                };
                let sd = SessionDescription {
                    r#type: sd_type,
                    sdp: data.sdp.sdp,
                };
                let relay = actr_protocol::ActrRelay {
                    source: actor_id,
                    credential,
                    target,
                    payload: Some(actr_relay::Payload::SessionDescription(sd)),
                };
                let envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: self.signaling.next_envelope_id(),
                    reply_for: None,
                    timestamp: SignalingClient::now_timestamp(),
                    traceparent: None,
                    tracestate: None,
                    flow: Some(signaling_envelope::Flow::ActrRelay(relay)),
                };
                self.signaling.send_envelope(envelope).await?;
            }
            "ice_candidate" => {
                let data: IceCandidateEvent = serde_json::from_value(event.data.clone())
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;

                // Resolve target: server uses peerId from event, client uses discover_target
                let target = if self.is_server {
                    resolve_target(&event.data)?
                } else {
                    self.discover_target().await?
                };

                let ice = IceCandidate {
                    candidate: data.candidate.candidate,
                    sdp_mid: data.candidate.sdp_mid,
                    sdp_mline_index: data.candidate.sdp_mline_index,
                    username_fragment: data.candidate.username_fragment,
                };
                let relay = actr_protocol::ActrRelay {
                    source: actor_id,
                    credential,
                    target,
                    payload: Some(actr_relay::Payload::IceCandidate(ice)),
                };
                let envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: self.signaling.next_envelope_id(),
                    reply_for: None,
                    timestamp: SignalingClient::now_timestamp(),
                    traceparent: None,
                    tracestate: None,
                    flow: Some(signaling_envelope::Flow::ActrRelay(relay)),
                };
                self.signaling.send_envelope(envelope).await?;
            }
            "datachannel_open" => {
                let data: DataChannelEvent = serde_json::from_value(event.data)
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                log::info!(
                    "[SW] datachannel_open: peer={} channel_id={}",
                    data.peer_id,
                    data.channel_id
                );
                self.mark_channel_open(&data.peer_id, data.channel_id);
                self.flush_pending_data(&data.peer_id, data.channel_id)?;

                // ICE restart completion: when datachannel re-opens after restart,
                // mark the restart as completed and reset the attempt counter.
                if self.ice_restart_inflight.get(&data.peer_id) == Some(&true) {
                    log::info!(
                        "[SW] ICE restart completed: peer={} (datachannel re-opened)",
                        data.peer_id
                    );
                    self.ice_restart_inflight
                        .insert(data.peer_id.clone(), false);
                    self.ice_restart_attempts.remove(&data.peer_id);
                }
            }
            "datachannel_close" => {
                let data: DataChannelEvent = serde_json::from_value(event.data)
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                log::info!(
                    "[SW] datachannel_close: peer={} channel_id={}",
                    data.peer_id,
                    data.channel_id
                );
                // Remove the channel from open set
                if let Some(channels) = self.open_channels.get_mut(&data.peer_id) {
                    channels.remove(&data.channel_id);
                }
            }
            "connection_state_changed" => {
                #[derive(Deserialize)]
                struct ConnectionStateEvent {
                    #[serde(rename = "peerId")]
                    peer_id: String,
                    state: String,
                }
                let data: ConnectionStateEvent = serde_json::from_value(event.data)
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;

                log::info!(
                    "[SW] connection_state_changed: peer={} state={}",
                    data.peer_id,
                    data.state
                );

                self.peer_connection_states
                    .insert(data.peer_id.clone(), data.state.clone());

                match data.state.as_str() {
                    "disconnected" => {
                        // Only the offerer should initiate ICE restart
                        let is_offerer = self
                            .role_assignments
                            .get(&data.peer_id)
                            .copied()
                            .unwrap_or(false);

                        if is_offerer {
                            self.trigger_ice_restart(&data.peer_id)?;
                        } else {
                            log::info!(
                                "[SW] ICE restart: not offerer for peer={}, waiting for remote restart",
                                data.peer_id
                            );
                        }
                    }
                    "failed" => {
                        // "failed" means the connection is irrecoverable via ICE restart.
                        // Immediately clean up all peer state and force re-discovery.
                        log::warn!(
                            "[SW] connection_state_changed: peer={} FAILED — cleaning up immediately",
                            data.peer_id
                        );
                        self.send_webrtc_command("close_peer", &data.peer_id, JsValue::NULL)?;
                        self.ice_restart_inflight.remove(&data.peer_id);
                        self.ice_restart_attempts.remove(&data.peer_id);
                        self.known_peers.remove(&data.peer_id);
                        self.open_channels.remove(&data.peer_id);
                        self.pending_channel_data.remove(&data.peer_id);
                        self.role_negotiated.remove(&data.peer_id);
                        self.role_assignments.remove(&data.peer_id);
                        self.peer_connection_states.remove(&data.peer_id);
                        if !self.is_server {
                            self.target_id = None;
                            log::info!(
                                "[SW] connection failed: cleared target_id for re-discovery"
                            );
                        }
                        // Notify DOM about connection failure so pending RPCs can fail fast
                        self.notify_connection_failure(&data.peer_id)?;
                    }
                    "connected" => {
                        // Connection recovered, clear any restart state
                        if self.ice_restart_inflight.get(&data.peer_id) == Some(&true) {
                            log::info!(
                                "[SW] ICE restart: connection recovered for peer={}",
                                data.peer_id
                            );
                            self.ice_restart_inflight
                                .insert(data.peer_id.clone(), false);
                            self.ice_restart_attempts.remove(&data.peer_id);
                        }
                    }
                    "closed" => {
                        // Peer is fully closed, clean up all associated state
                        self.ice_restart_inflight.remove(&data.peer_id);
                        self.ice_restart_attempts.remove(&data.peer_id);
                        self.known_peers.remove(&data.peer_id);
                        self.open_channels.remove(&data.peer_id);
                        self.pending_channel_data.remove(&data.peer_id);
                        self.role_negotiated.remove(&data.peer_id);
                        self.role_assignments.remove(&data.peer_id);
                        self.peer_connection_states.remove(&data.peer_id);
                        // Clear cached target_id to force re-discovery on next RPC
                        if !self.is_server {
                            self.target_id = None;
                            log::info!(
                                "[SW] connection closed: cleared target_id for re-discovery"
                            );
                        }
                    }
                    _ => {}
                }
            }
            "ice_restart_local_description" => {
                // ICE restart offer generated by DOM, send to remote peer
                let data: LocalDescriptionEvent = serde_json::from_value(event.data.clone())
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;

                let target = if self.is_server {
                    resolve_target(&event.data)?
                } else {
                    self.discover_target().await?
                };

                log::info!(
                    "[SW] Sending ICE restart offer to peer={}",
                    target.to_string_repr()
                );

                // Use ICE_RESTART_OFFER type (3) for the session description
                let sd = SessionDescription {
                    r#type: session_description::Type::IceRestartOffer as i32,
                    sdp: data.sdp.sdp,
                };
                let relay = actr_protocol::ActrRelay {
                    source: actor_id,
                    credential,
                    target,
                    payload: Some(actr_relay::Payload::SessionDescription(sd)),
                };
                let envelope = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: self.signaling.next_envelope_id(),
                    reply_for: None,
                    timestamp: SignalingClient::now_timestamp(),
                    traceparent: None,
                    tracestate: None,
                    flow: Some(signaling_envelope::Flow::ActrRelay(relay)),
                };
                self.signaling.send_envelope(envelope).await?;
            }
            "command_error" => {
                #[derive(Deserialize)]
                struct CommandErrorEvent {
                    #[serde(rename = "peerId")]
                    peer_id: String,
                    action: String,
                    error: String,
                }
                if let Ok(data) = serde_json::from_value::<CommandErrorEvent>(event.data) {
                    log::warn!(
                        "[SW] command_error: peer={} action={} error={}",
                        data.peer_id,
                        data.action,
                        data.error
                    );
                    // If the error is about a missing peer, clean up stale state
                    if data.error.contains("not found") {
                        self.known_peers.remove(&data.peer_id);
                        self.open_channels.remove(&data.peer_id);
                        self.role_negotiated.remove(&data.peer_id);
                        self.role_assignments.remove(&data.peer_id);
                        self.ice_restart_inflight.remove(&data.peer_id);
                        self.ice_restart_attempts.remove(&data.peer_id);
                        self.peer_connection_states.remove(&data.peer_id);
                        log::info!("[SW] cleaned up stale peer state for peer={}", data.peer_id);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// 处理 Fast Path 数据
    ///
    /// 按照文档架构：
    /// - RPC 响应（有 pending request）→ 直接处理（Fast Path ~1-3ms）
    /// - RPC 请求（入站请求）→ Mailbox → MailboxProcessor（State Path ~30-40ms）
    fn handle_fast_path(&mut self, payload: FastPathPayload) -> Result<(), JsValue> {
        let (_, channel_id) = parse_peer_and_channel(&payload.stream_id);

        if matches!(channel_id, 2 | 3) {
            return self.handle_data_stream(payload);
        }

        let envelope = RpcEnvelope::decode(&payload.data[..])
            .map_err(|e| JsValue::from_str(&format!("Failed to decode RpcEnvelope: {e}")))?;

        // 检查是否是我们发送的请求的响应
        let is_response = self.pending_rpcs.contains_key(&envelope.request_id);

        if is_response {
            // Fast Path：直接处理响应
            log::debug!(
                "[SW] Fast Path: response for request_id={}",
                envelope.request_id
            );
            self.handle_rpc_response(envelope)
        } else {
            // State Path：通过 Dispatcher 进入 Mailbox → MailboxProcessor → ServiceHandler
            // 按照文档架构，所有入站 RPC 请求必须经过 Mailbox 持久化和串行调度
            log::info!(
                "[SW] State Path: request route_key={} request_id={} → Mailbox",
                envelope.route_key,
                envelope.request_id
            );

            // 将 stream_id 作为 from 传递给 Mailbox，以便 MailboxProcessor 知道响应目标
            let stream_id_bytes = payload.stream_id.as_bytes().to_vec();
            let message = MessageFormat::new(PayloadType::RpcReliable, Bytes::from(payload.data));

            CLIENTS.with(|cell| {
                if let Some(ctx) = cell.borrow().get(&self.client_id) {
                    let dispatcher = Rc::clone(&ctx.dispatcher);
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = dispatcher.dispatch(stream_id_bytes, message).await {
                            log::error!("[SW] Dispatcher enqueue error: {}", e);
                        }
                    });
                } else {
                    log::warn!(
                        "[SW] No dispatcher available for client_id={}",
                        self.client_id
                    );
                }
            });

            Ok(())
        }
    }

    fn handle_data_stream(&mut self, payload: FastPathPayload) -> Result<(), JsValue> {
        let (logical_stream_id, data) = decode_stream_payload(&payload.data)?;

        let handler = CLIENTS.with(|cell| {
            cell.borrow().get(&self.client_id).and_then(|ctx| {
                ctx.stream_handlers
                    .borrow()
                    .get(&logical_stream_id)
                    .cloned()
            })
        });

        if let Some(handler) = handler {
            log::info!(
                "[SW] Fast Path stream dispatch: stream_id={} bytes={}",
                logical_stream_id,
                data.len()
            );
            (handler.borrow_mut())(data);
        } else {
            log::warn!(
                "[SW] No stream handler registered: stream_id={} peer_stream={}",
                logical_stream_id,
                payload.stream_id
            );
        }

        Ok(())
    }

    fn is_channel_open(&self, peer_id: &str, channel_id: u32) -> bool {
        self.open_channels
            .get(peer_id)
            .map(|channels| channels.contains(&channel_id))
            .unwrap_or(false)
    }

    fn mark_channel_open(&mut self, peer_id: &str, channel_id: u32) {
        self.open_channels
            .entry(peer_id.to_string())
            .or_default()
            .insert(channel_id);
    }

    fn queue_channel_data(&mut self, peer_id: &str, data: Vec<u8>) {
        self.pending_channel_data
            .entry(peer_id.to_string())
            .or_default()
            .push(data);
    }

    fn flush_pending_data(&mut self, peer_id: &str, channel_id: u32) -> Result<(), JsValue> {
        if channel_id != 0 {
            return Ok(());
        }
        let Some(pending) = self.pending_channel_data.remove(peer_id) else {
            return Ok(());
        };
        for data in pending {
            self.send_channel_data(peer_id, channel_id, &data)?;
        }
        Ok(())
    }

    fn send_channel_data(
        &self,
        peer_id: &str,
        channel_id: u32,
        data: &[u8],
    ) -> Result<(), JsValue> {
        log::info!(
            "[SW] send_channel_data: peer={} channel_id={} bytes={}",
            peer_id,
            channel_id,
            data.len()
        );
        let js_data: JsValue = js_sys::Uint8Array::from(data).into();

        let payload = SendDataPayload {
            channel_id,
            data: js_data,
        };

        let payload_js = serde_wasm_bindgen::to_value(&payload).map_err(|e| {
            JsValue::from_str(&format!("Failed to serialize send_data payload: {}", e))
        })?;

        self.send_webrtc_command("send_data", peer_id, payload_js)
    }

    /// Notify DOM about a connection failure so pending RPCs can fail fast.
    ///
    /// Sends error responses for all pending RPCs and a connection_failed
    /// event to the DOM, allowing the JS layer to immediately reject
    /// outstanding promises instead of waiting for the 30s timeout.
    fn notify_connection_failure(&mut self, peer_id: &str) -> Result<(), JsValue> {
        // 1. Fail all pending RPCs with a connection error
        let pending_entries: Vec<(String, PendingRpcTarget)> = self.pending_rpcs.drain().collect();
        let count = pending_entries.len();
        for (request_id, target) in &pending_entries {
            match target {
                PendingRpcTarget::Dom => {
                    let response = SwMessage {
                        msg_type: "control_response",
                        payload: RpcResponsePayload {
                            request_id: request_id.clone(),
                            data: JsValue::NULL,
                            error: Some(RpcError {
                                code: 503,
                                message: format!(
                                    "WebRTC connection failed for peer {}, request cancelled",
                                    peer_id
                                ),
                            }),
                        },
                    };
                    let msg_js_value = serde_wasm_bindgen::to_value(&response).map_err(|e| {
                        JsValue::from_str(&format!("Failed to serialize error response: {}", e))
                    })?;
                    let _ = self.send_dom_message(&msg_js_value);
                }
                PendingRpcTarget::Internal => {
                    // Internal RPCs: resolved via System/HostGate error handling
                    CLIENTS.with(|cell| {
                        if let Some(ctx) = cell.borrow().get(&self.client_id) {
                            ctx.system.handle_remote_response(request_id, Bytes::new());
                        }
                    });
                }
            }
        }

        if count > 0 {
            log::info!(
                "[SW] notify_connection_failure: rejected {} pending RPCs for peer={}",
                count,
                peer_id
            );
        }

        Ok(())
    }

    /// Trigger an ICE restart for the given peer.
    ///
    /// Only the offerer side should call this. Implements deduplication:
    /// - Skips if an ICE restart is already in-flight
    /// - Skips if max retries exceeded (will close peer instead)
    fn trigger_ice_restart(&mut self, peer_id: &str) -> Result<(), JsValue> {
        // Dedup: skip if already in-flight
        if self.ice_restart_inflight.get(peer_id) == Some(&true) {
            log::info!(
                "[SW] ICE restart: already in-flight for peer={}, skipping",
                peer_id
            );
            return Ok(());
        }

        // Check retry count
        let attempts = self
            .ice_restart_attempts
            .entry(peer_id.to_string())
            .or_insert(0);
        if *attempts >= ICE_RESTART_MAX_RETRIES {
            log::warn!(
                "[SW] ICE restart: max retries ({}) exceeded for peer={}, closing connection",
                ICE_RESTART_MAX_RETRIES,
                peer_id
            );
            // Clean up: close the peer connection
            self.send_webrtc_command("close_peer", peer_id, JsValue::NULL)?;
            self.known_peers.remove(peer_id);
            self.open_channels.remove(peer_id);
            self.ice_restart_inflight.remove(peer_id);
            self.ice_restart_attempts.remove(peer_id);
            self.peer_connection_states.remove(peer_id);
            self.role_negotiated.remove(peer_id);
            self.role_assignments.remove(peer_id);
            // Clear cached target_id to force re-discovery on next RPC
            if !self.is_server {
                self.target_id = None;
                log::info!("[SW] ICE restart: cleared target_id cache for re-discovery");
            }
            // Fail-fast: reject all pending RPCs immediately
            self.notify_connection_failure(peer_id)?;
            return Ok(());
        }

        *attempts += 1;
        let attempt = *attempts;
        self.ice_restart_inflight.insert(peer_id.to_string(), true);

        log::info!(
            "[SW] ICE restart: initiating for peer={} (attempt {}/{})",
            peer_id,
            attempt,
            ICE_RESTART_MAX_RETRIES
        );

        // Send create_ice_restart_offer command to DOM
        self.send_webrtc_command("create_ice_restart_offer", peer_id, JsValue::NULL)?;

        // Schedule a timeout to check if restart completed
        let peer_id_owned = peer_id.to_string();
        let runtime = CLIENTS.with(|cell| {
            cell.borrow()
                .get(&self.client_id)
                .map(|ctx| Rc::clone(&ctx.runtime))
        });
        if let Some(runtime) = runtime {
            wasm_bindgen_futures::spawn_local(async move {
                // Wait for the restart timeout
                TimeoutFuture::new(ICE_RESTART_TIMEOUT_MS).await;

                let mut rt = runtime.lock().await;
                // Check if restart is still in-flight (i.e., not completed)
                if rt.ice_restart_inflight.get(&peer_id_owned) == Some(&true) {
                    log::warn!(
                        "[SW] ICE restart: timeout for peer={}, will retry",
                        peer_id_owned
                    );
                    // Mark as not in-flight so next trigger can proceed
                    rt.ice_restart_inflight.insert(peer_id_owned.clone(), false);

                    // Check current connection state - if still disconnected/failed, retry
                    let state = rt
                        .peer_connection_states
                        .get(&peer_id_owned)
                        .cloned()
                        .unwrap_or_default();
                    if state == "disconnected" || state == "failed" {
                        // Calculate backoff delay
                        let attempt = rt
                            .ice_restart_attempts
                            .get(&peer_id_owned)
                            .copied()
                            .unwrap_or(0);
                        let backoff_ms = std::cmp::min(
                            ICE_RESTART_INITIAL_BACKOFF_MS * (1 << (attempt.saturating_sub(1))),
                            ICE_RESTART_MAX_BACKOFF_MS,
                        );
                        log::info!(
                            "[SW] ICE restart: scheduling retry in {}ms for peer={}",
                            backoff_ms,
                            peer_id_owned
                        );

                        // Drop the lock before sleeping
                        drop(rt);
                        TimeoutFuture::new(backoff_ms).await;

                        let mut rt = runtime.lock().await;
                        // Re-check state after backoff
                        let state = rt
                            .peer_connection_states
                            .get(&peer_id_owned)
                            .cloned()
                            .unwrap_or_default();
                        if state == "disconnected" || state == "failed" {
                            let _ = rt.trigger_ice_restart(&peer_id_owned);
                        }
                    }
                }
            });
        }

        Ok(())
    }
}

/// Type alias for a unified service handler function (UnifiedDispatcher).
///
/// Given (route_key, request_bytes, context), returns a future that resolves
/// to the response bytes or an error string.
///
/// The handler dispatches based on route_key prefix to local or remote handlers.
/// For remote calls, the handler uses `ctx.call_raw()` / `ctx.discover()`.
///
/// # Parameters
/// - `route_key`: Full route key (e.g., `"echo.EchoService.Echo"`)
/// - `request_bytes`: Serialized protobuf request payload
/// - `ctx`: WebContext providing communication capabilities (call_raw, discover, etc.)
pub type ServiceHandlerFn = Rc<
    dyn Fn(
        &str,
        &[u8],
        Rc<RuntimeContext>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>>>>,
>;

/// Per-client context stored in the SW.
/// Each browser tab gets its own independent client context with
/// its own signaling connection, actor registration, and WebRTC state.
struct ClientContext {
    runtime: Rc<Mutex<SwRuntime>>,
    system: Rc<crate::System>,
    dispatcher: Rc<InboundPacketDispatcher>,
    /// PeerGate — 跨节点传输适配器
    peer_gate: Arc<crate::outbound::PeerGate>,
    /// PeerTransport — 管理每个 Dest 的 DestTransport
    transport_manager: Arc<crate::transport::PeerTransport>,
    /// DataStream 回调注册表（每个浏览器 tab 独立）
    stream_handlers: Rc<RefCell<HashMap<String, StreamHandler>>>,
}

/// SwRuntimeBridge - RuntimeBridge 的实现
///
/// 连接 RuntimeContext 和 SwRuntime / System / PeerGate，
/// 为 handler 内的 `ctx.call_raw()` / `ctx.discover()` 提供底层能力。
struct SwRuntimeBridge {
    runtime: Rc<Mutex<SwRuntime>>,
    peer_gate: Arc<crate::outbound::PeerGate>,
    client_id: String,
}

#[async_trait::async_trait(?Send)]
impl RuntimeBridge for SwRuntimeBridge {
    fn register_pending_rpc(&self, request_id: String) {
        // 同步注册：使用 futures::lock::Mutex::try_lock
        // 在 WASM 单线程环境中，如果锁不可用说明存在 bug
        if let Some(mut rt) = self.runtime.try_lock() {
            rt.register_pending_rpc(request_id, PendingRpcTarget::Internal);
        } else {
            log::error!("[SwRuntimeBridge] Failed to lock runtime for pending RPC registration");
        }
    }

    async fn discover_target(&self, target_type: &ActrType) -> actr_protocol::ActorResult<ActrId> {
        let mut rt = self.runtime.lock().await;
        rt.discover_target_for_type(target_type)
            .await
            .map_err(|e| actr_protocol::ActrError::Unavailable(format!("Discover failed: {:?}", e)))
    }

    async fn ensure_connection(&self, target_id: &ActrId) -> actr_protocol::ActorResult<()> {
        let mut rt = self.runtime.lock().await;
        let peer_id = rt.ensure_peer_with_retry().await.map_err(|e| {
            actr_protocol::ActrError::Unavailable(format!("Failed to ensure peer: {:?}", e))
        })?;
        self.peer_gate
            .register_actor(target_id.clone(), actr_web_common::Dest::Peer(peer_id));
        Ok(())
    }

    fn register_stream_handler(
        &self,
        stream_id: String,
        callback: Box<dyn FnMut(Bytes) + 'static>,
    ) -> actr_protocol::ActorResult<()> {
        CLIENTS.with(|cell| {
            let map = cell.borrow();
            let Some(ctx) = map.get(&self.client_id) else {
                return Err(actr_protocol::ActrError::Unavailable(format!(
                    "Client context not found: {}",
                    self.client_id
                )));
            };

            ctx.stream_handlers
                .borrow_mut()
                .insert(stream_id.clone(), Rc::new(RefCell::new(callback)));

            log::info!(
                "[SW] Stream handler registered: client_id={} stream_id={}",
                self.client_id,
                stream_id
            );
            Ok(())
        })
    }

    fn unregister_stream_handler(&self, stream_id: &str) -> actr_protocol::ActorResult<()> {
        CLIENTS.with(|cell| {
            let map = cell.borrow();
            let Some(ctx) = map.get(&self.client_id) else {
                return Err(actr_protocol::ActrError::Unavailable(format!(
                    "Client context not found: {}",
                    self.client_id
                )));
            };

            ctx.stream_handlers.borrow_mut().remove(stream_id);
            log::info!(
                "[SW] Stream handler unregistered: client_id={} stream_id={}",
                self.client_id,
                stream_id
            );
            Ok(())
        })
    }
}

fn parse_peer_and_channel(stream_id: &str) -> (String, u32) {
    if let Some(last_colon) = stream_id.rfind(':') {
        let peer = &stream_id[..last_colon];
        let channel = stream_id[last_colon + 1..].parse::<u32>().unwrap_or(0);
        (peer.to_string(), channel)
    } else {
        (stream_id.to_string(), 0)
    }
}

fn decode_stream_payload(data: &[u8]) -> Result<(String, Bytes), JsValue> {
    if data.len() < 4 {
        return Err(JsValue::from_str(
            "Invalid DataStream payload: missing stream_id length",
        ));
    }

    let stream_id_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if data.len() < 4 + stream_id_len {
        return Err(JsValue::from_str(
            "Invalid DataStream payload: truncated stream_id",
        ));
    }

    let logical_stream_id = String::from_utf8(data[4..4 + stream_id_len].to_vec())
        .map_err(|e| JsValue::from_str(&format!("Invalid DataStream stream_id: {}", e)))?;

    Ok((
        logical_stream_id,
        Bytes::copy_from_slice(&data[4 + stream_id_len..]),
    ))
}

thread_local! {
    /// Map of client_id → ClientContext for multi-tab support.
    /// Each browser tab registers with a unique client_id.
    static CLIENTS: RefCell<HashMap<String, Rc<ClientContext>>> = RefCell::new(HashMap::new());
    static GLOBAL_INITIALIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SERVICE_HANDLER: RefCell<Option<ServiceHandlerFn>> = const { RefCell::new(None) };
}

/// Register a unified service handler (UnifiedDispatcher pattern).
///
/// The handler receives (route_key, request_bytes, ctx) and dispatches to
/// local or remote handlers based on route_key prefix.
///
/// # Example
/// ```ignore
/// register_service_handler(Rc::new(|route_key, bytes, ctx| {
///     Box::pin(async move {
///         if route_key.starts_with("local.") {
///             handle_local(route_key, bytes).await
///         } else {
///             // Forward to remote via ctx.call_raw()
///             let target = ctx.discover(&target_type).await.map_err(|e| e.to_string())?;
///             ctx.call_raw(&target, route_key, bytes, 30000).await.map_err(|e| e.to_string())
///         }
///     })
/// }));
/// ```
pub fn register_service_handler(handler: ServiceHandlerFn) {
    SERVICE_HANDLER.with(|cell| {
        *cell.borrow_mut() = Some(handler);
    });
    log::info!("[SW] Service handler registered");
}

#[wasm_bindgen]
pub fn init_global() -> Result<(), JsValue> {
    let first_init = GLOBAL_INITIALIZED.with(|cell| {
        if cell.get() {
            false
        } else {
            cell.set(true);
            true
        }
    });

    if first_init {
        // 设置 panic hook，确保 panic 信息输出到控制台
        console_error_panic_hook::set_once();

        // 初始化日志
        wasm_logger::init(wasm_logger::Config::default());

        // 初始化生命周期管理
        let lifecycle = crate::SwLifecycleManager::new();
        if let Err(e) = lifecycle.init() {
            log::error!("Failed to initialize lifecycle manager: {:?}", e);
        }
        log::info!("[SW] Global initialization complete");
    }

    Ok(())
}

/// Register a new client (browser tab) with the SW runtime.
///
/// Each call creates an independent runtime with its own signaling connection,
/// actor registration, and WebRTC state. This enables multiple browser tabs
/// to work simultaneously without interfering with each other.
#[wasm_bindgen]
pub async fn register_client(
    client_id: String,
    config: JsValue,
    port: MessagePort,
) -> Result<(), JsValue> {
    // Ensure global init
    init_global()?;

    log::info!("[SW] register_client: client_id={}", client_id);

    let config: SwConfig = serde_wasm_bindgen::from_value(config)?;
    log::info!(
        "[SW] SwConfig parsed: client_id={} acl_allow_types={:?}, is_server={}",
        client_id,
        config.acl_allow_types,
        config.is_server
    );
    let mut runtime = SwRuntime::new(client_id.clone(), config).await?;
    runtime.register().await?;

    // Set DOM port
    runtime.dom_port = Some(port);

    // 获取 actor_id 用于 System
    let actor_id = runtime.actor_id.clone();

    let runtime = Rc::new(Mutex::new(runtime));

    // ==================== State Path 初始化 ====================
    // 按照文档架构: InboundDispatcher → Mailbox → MailboxProcessor → Scheduler → Actor
    log::info!("[SW] [{}] Initializing State Path components...", client_id);

    // 1. 创建 Mailbox（IndexedDB）
    let mailbox: Rc<dyn Mailbox> = Rc::new(
        IndexedDbMailbox::new()
            .await
            .map_err(|e| JsValue::from_str(&format!("Failed to create Mailbox: {}", e)))?,
    );

    // 2. 创建 MailboxProcessor（同时获取事件驱动的 notifier）
    let (mut processor, notifier) = MailboxProcessor::new(mailbox.clone(), 10);

    // 3. 创建 InboundPacketDispatcher（复用 IndexedDB 后端，附带 notifier）
    let mailbox_arc: Arc<dyn Mailbox> = Arc::from(IndexedDbMailbox::new().await.map_err(|e| {
        JsValue::from_str(&format!("Failed to create Mailbox for dispatcher: {}", e))
    })?);
    let dispatcher =
        Rc::new(InboundPacketDispatcher::new(mailbox_arc.clone()).with_notifier(notifier));

    // 4. 创建 Scheduler（串行调度器，保证同一 Actor 消息顺序执行）
    let scheduler = Scheduler::new();

    // 5. 设置 Scheduler 的 Actor 处理回调
    //    按照文档：Scheduler → Actor 业务逻辑 → 响应返回
    let runtime_for_scheduler = Rc::clone(&runtime);
    let client_id_for_scheduler = client_id.clone();
    scheduler.set_handler(Rc::new(move |record: MessageRecord| {
        let runtime = Rc::clone(&runtime_for_scheduler);
        let client_id = client_id_for_scheduler.clone();
        Box::pin(async move {
            // 解析消息（所有进入 Mailbox 的消息都是入站 RPC 请求）
            let envelope = RpcEnvelope::decode(&record.payload[..])
                .map_err(|e| actr_web_common::WebError::Protocol(format!("Failed to decode RpcEnvelope: {}", e)))?;

            log::info!(
                "[Scheduler] Processing RPC request: request_id={}, route_key={}",
                envelope.request_id,
                envelope.route_key
            );

            // 获取已注册的全局 service handler
            let handler = SERVICE_HANDLER.with(|cell| cell.borrow().as_ref().map(Rc::clone));

            if let Some(handler) = handler {
                let route_key = envelope.route_key.clone();
                let request_id = envelope.request_id.clone();
                let is_tell = envelope.timeout_ms == 0; // tell() 设置 timeout_ms=0，表示单向消息
                let request_bytes = envelope
                    .payload
                    .as_ref()
                    .map(|p| p.to_vec())
                    .unwrap_or_default();
                let stream_id = String::from_utf8(record.from.clone()).unwrap_or_default();
                let caller_id = envelope
                    .metadata
                    .iter()
                    .find(|entry| entry.key == "sender_actr_id")
                    .and_then(|entry| ActrId::from_string_repr(&entry.value).ok());
                let (peer_id, _channel_id) = parse_peer_and_channel(&stream_id);

                log::info!(
                    "[Scheduler] Dispatching to handler: route_key={} request_id={} is_tell={}",
                    route_key,
                    request_id,
                    is_tell
                );

                // Build RuntimeContext for the handler (server-side: uses Peer gate for outbound)
                // Look up system/peer_gate from CLIENTS (initialized after scheduler setup)
                let (outgate, bridge) = CLIENTS.with(|cell| {
                    let map = cell.borrow();
                    if let Some(ctx) = map.get(&client_id) {
                        if let Some(caller_id) = caller_id.clone() {
                            ctx.peer_gate.register_actor(
                                caller_id,
                                actr_web_common::Dest::Peer(peer_id.clone()),
                            );
                        }
                        let outgate = ctx.system.outgate().unwrap_or_else(|| {
                            Gate::host(Arc::clone(ctx.system.host_gate()))
                        });
                        let bridge: Rc<dyn RuntimeBridge> = Rc::new(SwRuntimeBridge {
                            runtime: Rc::clone(&runtime),
                            peer_gate: Arc::clone(&ctx.peer_gate),
                            client_id: client_id.clone(),
                        });
                        (outgate, bridge)
                    } else {
                        // Fallback: Host gate with no bridge
                        log::error!("[Scheduler] Client context not found for {}", client_id);
                        let gate = Gate::host(Arc::new(crate::outbound::HostGate::new()));
                        let bridge: Rc<dyn RuntimeBridge> = Rc::new(SwRuntimeBridge {
                            runtime: Rc::clone(&runtime),
                            peer_gate: Arc::new(crate::outbound::PeerGate::new(
                                Arc::new(crate::transport::PeerTransport::new(
                                    client_id.clone(),
                                    Arc::new(crate::transport::WebWireBuilder::new()),
                                )),
                            )),
                            client_id: client_id.clone(),
                        });
                        (gate, bridge)
                    }
                });
                let actor_id = {
                    let rt = runtime.lock().await;
                    rt.actor_id.clone().unwrap_or_default()
                };
                let handler_ctx: Rc<RuntimeContext> = Rc::new(
                    RuntimeContext::new(
                        actor_id,
                        caller_id,
                        envelope.traceparent.clone().unwrap_or_default(),
                        envelope.tracestate.clone().unwrap_or_default(),
                        request_id.clone(),
                        outgate,
                    )
                    .with_bridge(bridge),
                );

                // 调用 Actor 业务逻辑
                let result = handler(&route_key, &request_bytes, handler_ctx).await;

                // tell() 是单向消息（fire-and-forget），无需构建或发送响应
                if is_tell {
                    match result {
                        Ok(_) => log::debug!(
                            "[Scheduler] Tell handled successfully: request_id={}",
                            request_id
                        ),
                        Err(err) => log::error!(
                            "[Scheduler] Tell handler error (no response sent): request_id={} error={}",
                            request_id,
                            err
                        ),
                    }
                } else {
                    // call() 请求-响应模式：构建并发送响应
                    let response_envelope = match result {
                        Ok(response_bytes) => {
                            log::info!(
                                "[Scheduler] Service handler success: request_id={} response_len={}",
                                request_id,
                                response_bytes.len()
                            );
                            RpcEnvelope {
                                route_key: route_key.clone(),
                                payload: Some(Bytes::from(response_bytes)),
                                error: None,
                                traceparent: None,
                                tracestate: None,
                                request_id: request_id.clone(),
                                metadata: vec![],
                                timeout_ms: 0,
                            }
                        }
                        Err(err) => {
                            log::error!(
                                "[Scheduler] Service handler error: request_id={} error={}",
                                request_id,
                                err
                            );
                            RpcEnvelope {
                                route_key: route_key.clone(),
                                payload: None,
                                error: Some(actr_protocol::ErrorResponse {
                                    code: 500,
                                    message: err,
                                }),
                                traceparent: None,
                                tracestate: None,
                                request_id: request_id.clone(),
                                metadata: vec![],
                                timeout_ms: 0,
                            }
                        }
                    };

                    let data = response_envelope.encode_to_vec();

                    // 解析发送方的 peer_id 和 channel_id（从 stream_id 中提取）
                    let (peer_id, channel_id) = if let Some(last_colon) = stream_id.rfind(':') {
                        let peer = &stream_id[..last_colon];
                        let ch = stream_id[last_colon + 1..].parse::<u32>().unwrap_or(0);
                        (peer.to_string(), ch)
                    } else {
                        (stream_id.clone(), 0u32)
                    };

                    log::info!(
                        "[Scheduler] Sending RPC response: request_id={} peer={} channel={} bytes={}",
                        request_id,
                        peer_id,
                        channel_id,
                        data.len()
                    );

                    // 通过 SwRuntime 发送响应回远程 peer
                    let rt = runtime.lock().await;
                    if let Err(e) = rt.send_channel_data(&peer_id, channel_id, &data) {
                        log::error!(
                            "[Scheduler] Failed to send RPC response: request_id={} error={:?}",
                            request_id,
                            e
                        );
                    }
                }
            } else {
                log::warn!(
                    "[Scheduler] No service handler for incoming RPC request: route_key={}",
                    envelope.route_key
                );
            }

            Ok(())
        })
    }));

    // 6. 设置 MailboxProcessor 的处理回调
    //    按照文档：MailboxProcessor → Scheduler（路由到对应 Actor 的串行队列）
    let scheduler_for_processor = scheduler.clone();
    let local_actor_id_for_processor = actor_id.clone().unwrap_or_else(|| {
        log::warn!("[SW] actor_id not set after register, using default for Scheduler");
        ActrId::default()
    });
    processor.set_handler(Rc::new(move |record: MessageRecord| {
        let sched = scheduler_for_processor.clone();
        let aid = local_actor_id_for_processor.clone();
        Box::pin(async move {
            sched.schedule(aid, record);
            Ok(())
        })
    }));

    // 7. 启动 Scheduler 和 MailboxProcessor
    scheduler.start();
    processor.start();

    // ==================== System 初始化 ====================

    let system = Rc::new(crate::System::new());
    if let Some(actor_id) = actor_id {
        system.set_local_actor_id(actor_id);
    }

    // 创建完整传输栈：PeerGate → PeerTransport → DestTransport → WirePool
    let wire_builder = Arc::new(crate::transport::WebWireBuilder::new());
    let transport_manager = Arc::new(crate::transport::PeerTransport::new(
        client_id.clone(),
        wire_builder,
    ));
    let peer_gate = Arc::new(crate::outbound::PeerGate::new(Arc::clone(
        &transport_manager,
    )));
    system.set_outgate(crate::outbound::Gate::peer(Arc::clone(&peer_gate)));

    system.init_message_handler();

    // ==================== Store ClientContext ====================

    let client_context = Rc::new(ClientContext {
        runtime: Rc::clone(&runtime),
        system: Rc::clone(&system),
        dispatcher: Rc::clone(&dispatcher),
        peer_gate: Arc::clone(&peer_gate),
        transport_manager: Arc::clone(&transport_manager),
        stream_handlers: Rc::new(RefCell::new(HashMap::new())),
    });

    CLIENTS.with(|cell| {
        cell.borrow_mut().insert(client_id.clone(), client_context);
    });

    // ==================== Background Loops ====================

    let runtime_for_loop = Rc::clone(&runtime);
    let signaling = runtime_for_loop.lock().await.signaling.clone();
    let client_id_for_loop = client_id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(env) = signaling.recv_inbound().await {
            if let Some(signaling_envelope::Flow::ActrRelay(relay)) = env.flow {
                let mut runtime = runtime_for_loop.lock().await;
                let _ = runtime.handle_actr_relay(relay);
            }
        }
        log::info!("[SW] [{}] Signaling relay loop ended", client_id_for_loop);
    });

    let runtime_for_heartbeat = Rc::clone(&runtime);
    let client_id_for_heartbeat = client_id.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut consecutive_failures: u32 = 0;
        loop {
            TimeoutFuture::new(25_000).await;

            // ---- try heartbeat ----
            let hb_result = {
                let rt = runtime_for_heartbeat.lock().await;
                rt.send_heartbeat().await
            };

            if hb_result.is_ok() {
                consecutive_failures = 0;
                continue;
            }

            // ---- heartbeat failed ----
            consecutive_failures += 1;
            log::warn!(
                "[SW] [{}] Heartbeat failed ({}/3): {:?}",
                client_id_for_heartbeat,
                consecutive_failures,
                hb_result.unwrap_err()
            );

            if consecutive_failures < 3 {
                continue;
            }

            // ---- 3 consecutive failures → reconnect ----
            log::info!(
                "[SW] [{}] Attempting signaling reconnection...",
                client_id_for_heartbeat
            );

            let reconnect_result = {
                let mut rt = runtime_for_heartbeat.lock().await;
                rt.reconnect_signaling().await
            };

            match reconnect_result {
                Ok(_) => {
                    log::info!(
                        "[SW] [{}] Signaling reconnected, spawning new relay loop",
                        client_id_for_heartbeat
                    );
                    consecutive_failures = 0;

                    // Spawn a fresh relay loop for the new signaling client.
                    let new_signaling = {
                        let rt = runtime_for_heartbeat.lock().await;
                        rt.signaling.clone()
                    };
                    let runtime_for_new_relay = Rc::clone(&runtime_for_heartbeat);
                    let cid = client_id_for_heartbeat.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        while let Some(env) = new_signaling.recv_inbound().await {
                            if let Some(signaling_envelope::Flow::ActrRelay(relay)) = env.flow {
                                let mut runtime = runtime_for_new_relay.lock().await;
                                let _ = runtime.handle_actr_relay(relay);
                            }
                        }
                        log::info!("[SW] [{}] Signaling relay loop ended (post-reconnect)", cid);
                    });
                }
                Err(e) => {
                    log::error!(
                        "[SW] [{}] Signaling reconnection failed: {:?}, stopping heartbeat",
                        client_id_for_heartbeat,
                        e
                    );
                    break;
                }
            }
        }
        log::info!("[SW] [{}] Heartbeat loop ended", client_id_for_heartbeat);
    });

    log::info!(
        "[SW] [{}] Client registration complete (State Path enabled)",
        client_id
    );
    Ok(())
}

/// Unregister a client (browser tab) from the SW runtime.
///
/// Closes the signaling WebSocket (so the signaling server removes
/// the actor from its ServiceRegistry) and removes the client context.
/// Background tasks (signaling relay, heartbeat) will naturally stop
/// when the signaling connection drops.
#[wasm_bindgen]
pub async fn unregister_client(client_id: String) {
    log::info!("[SW] unregister_client: client_id={}", client_id);
    let ctx = CLIENTS.with(|cell| cell.borrow_mut().remove(&client_id));
    if let Some(ctx) = ctx {
        ctx.stream_handlers.borrow_mut().clear();
        let rt = ctx.runtime.lock().await;
        rt.signaling.close();
        log::info!(
            "[SW] unregister_client: signaling WebSocket closed for client_id={}",
            client_id
        );
    }
}

/// 处理来自 DOM 的 RPC 控制请求
///
/// 消息流（Unified Dispatcher 模式）：
/// - 有 SERVICE_HANDLER: DOM → handler(route_key, payload, ctx) → response
///   - local route: handler 本地处理，可通过 ctx.call_raw() 调远程
///   - remote route: handler 通过 ctx.call_raw() 转发到远程 Actor
/// - 无 SERVICE_HANDLER: DOM → HostGate → Gate → WebRTC（旧路径，向后兼容）
#[wasm_bindgen]
pub async fn handle_dom_control(client_id: String, payload: JsValue) -> Result<(), JsValue> {
    let call: DomRpcCall = serde_wasm_bindgen::from_value(payload)?;

    if call.action != "rpc_call" {
        return Ok(());
    }

    // 获取 ClientContext
    let ctx = CLIENTS.with(|cell| cell.borrow().get(&client_id).map(Rc::clone));

    let Some(ctx) = ctx else {
        log::error!(
            "[SW] handle_dom_control: client not found client_id={}",
            client_id
        );
        return Err(JsValue::from_str("Client not registered"));
    };

    let system = &ctx.system;
    let runtime = &ctx.runtime;

    let route_key = call.request.route_key.clone();
    let payload_bytes = call.request.payload.clone();
    let request_id = call.request_id.clone();
    let timeout_ms = call.request.timeout.unwrap_or(30000);

    // Check if a service handler (UnifiedDispatcher) is registered
    let handler = SERVICE_HANDLER.with(|cell| cell.borrow().as_ref().map(Rc::clone));

    if let Some(handler) = handler {
        // ========== New path: UnifiedDispatcher (local + remote handler) ==========
        log::info!(
            "[SW] handle_dom_control: client_id={} route_key={} request_id={} (via handler)",
            client_id,
            route_key,
            request_id
        );

        // Build RuntimeContext for the handler
        let actor_id = {
            let rt = runtime.lock().await;
            rt.actor_id.clone().unwrap_or_default()
        };
        // Use Host Gate so call_raw goes through:
        // HostGate → MessageHandler → Gate::Peer → WebRTC
        let outgate = Gate::host(Arc::clone(system.host_gate()));
        let bridge: Rc<dyn RuntimeBridge> = Rc::new(SwRuntimeBridge {
            runtime: Rc::clone(runtime),
            peer_gate: Arc::clone(&ctx.peer_gate),
            client_id: client_id.clone(),
        });
        let handler_ctx: Rc<RuntimeContext> = Rc::new(
            RuntimeContext::new(
                actor_id,
                None,
                String::new(),
                String::new(),
                request_id.clone(),
                outgate,
            )
            .with_bridge(bridge),
        );

        // Spawn handler execution
        let runtime_for_response = Rc::clone(runtime);
        wasm_bindgen_futures::spawn_local(async move {
            let result = handler(&route_key, &payload_bytes, handler_ctx).await;

            // Send response back to DOM as control_response
            let rt = runtime_for_response.lock().await;
            match result {
                Ok(response_bytes) => {
                    let js_payload: JsValue =
                        js_sys::Uint8Array::from(response_bytes.as_slice()).into();
                    let response = SwMessage {
                        msg_type: "control_response",
                        payload: RpcResponsePayload {
                            request_id: request_id.clone(),
                            data: js_payload,
                            error: None,
                        },
                    };
                    if let Ok(msg) = serde_wasm_bindgen::to_value(&response) {
                        let _ = rt.send_dom_message(&msg);
                    }
                }
                Err(err) => {
                    log::error!(
                        "[SW] Handler error: request_id={} error={}",
                        request_id,
                        err
                    );
                    let response = SwMessage {
                        msg_type: "control_response",
                        payload: RpcResponsePayload {
                            request_id: request_id.clone(),
                            data: JsValue::NULL,
                            error: Some(RpcError {
                                code: 500,
                                message: err,
                            }),
                        },
                    };
                    if let Ok(msg) = serde_wasm_bindgen::to_value(&response) {
                        let _ = rt.send_dom_message(&msg);
                    }
                }
            }
        });
    } else {
        // ========== Legacy path: direct remote forwarding (backward compatible) ==========
        let envelope = RpcEnvelope {
            route_key: route_key.clone(),
            payload: Some(Bytes::from(payload_bytes)),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: request_id.clone(),
            metadata: vec![],
            timeout_ms: timeout_ms as i64,
        };

        log::info!(
            "[SW] handle_dom_control: client_id={} route_key={} request_id={} (legacy remote path)",
            client_id,
            route_key,
            request_id
        );

        // 获取目标 ActrId（从 Runtime 的 target_id）
        let target_id = {
            let rt = runtime.lock().await;
            rt.target_id.clone()
        };

        // 如果还没有 target_id，需要先发现目标
        let target_id = match target_id {
            Some(id) => id,
            None => {
                let mut rt = runtime.lock().await;
                rt.discover_target_with_retry().await.map_err(|e| {
                    log::error!("[SW] Failed to discover target: {:?}", e);
                    e
                })?
            }
        };

        // 确保 P2P 连接已建立，并注册 ActrId → Dest 映射
        {
            let mut rt = runtime.lock().await;
            let peer_id = rt.ensure_peer_with_retry().await.map_err(|e| {
                log::error!("[SW] Failed to ensure peer: {:?}", e);
                e
            })?;
            ctx.peer_gate
                .register_actor(target_id.clone(), actr_web_common::Dest::Peer(peer_id));
            rt.pending_rpcs
                .insert(request_id.clone(), PendingRpcTarget::Dom);
        }

        // 通过 HostGate 发送请求
        let host_gate = Arc::clone(system.host_gate());

        wasm_bindgen_futures::spawn_local({
            let request_id = request_id.clone();
            async move {
                match host_gate.send_request(&target_id, envelope).await {
                    Ok(_response) => {
                        log::info!(
                            "[SW] HostGate response received for request_id={}",
                            request_id
                        );
                    }
                    Err(e) => {
                        log::error!("[SW] HostGate send_request failed: {:?}", e);
                    }
                }
            }
        });
    }

    Ok(())
}

#[wasm_bindgen]
pub async fn handle_dom_webrtc_event(client_id: String, payload: JsValue) -> Result<(), JsValue> {
    let event: DomWebRtcEvent = serde_wasm_bindgen::from_value(payload)?;
    let runtime = CLIENTS.with(|cell| {
        cell.borrow()
            .get(&client_id)
            .map(|ctx| Rc::clone(&ctx.runtime))
    });
    if let Some(runtime) = runtime {
        wasm_bindgen_futures::spawn_local(async move {
            let mut rt = runtime.lock().await;
            let _ = rt.handle_webrtc_event(event).await;
        });
    } else {
        log::warn!(
            "[SW] handle_dom_webrtc_event: client not found client_id={}",
            client_id
        );
    }
    Ok(())
}

/// 注册来自 DOM 的专用 DataChannel MessagePort
///
/// DOM 在 DataChannel 建立后创建 MessageChannel 桥接：
/// 1. DOM: `port1 ↔ DataChannel` (双向转发)
/// 2. DOM: 将 `port2` 通过 Transferable 转移给 SW
/// 3. SW: 此函数接收 `port2`，创建 `WebRtcConnection`，注入到 `WirePool`
///
/// 注入后 DestTransport 的 send 循环通过 ReadyWatcher 被唤醒，
/// 后续出站数据直接经 `DataLane::PostMessage(port)` 零拷贝发送。
#[wasm_bindgen]
pub async fn register_datachannel_port(
    client_id: String,
    peer_id: String,
    port: MessagePort,
) -> Result<(), JsValue> {
    log::info!(
        "[SW] register_datachannel_port: client_id={} peer_id={}",
        client_id,
        peer_id
    );

    let ctx = CLIENTS.with(|cell| cell.borrow().get(&client_id).map(Rc::clone));

    let Some(ctx) = ctx else {
        log::error!(
            "[SW] register_datachannel_port: client not found client_id={}",
            client_id
        );
        return Err(JsValue::from_str("Client not registered"));
    };

    // 创建 WebRtcConnection 并设置 MessagePort
    let mut rtc_conn = crate::transport::WebRtcConnection::new(peer_id.clone());
    rtc_conn.set_datachannel_port(port);

    let wire_handle = crate::transport::WireHandle::WebRTC(rtc_conn);
    let dest = actr_web_common::Dest::Peer(peer_id.clone());

    // 注入到 PeerTransport 对应的 WirePool
    // 如果 DestTransport 尚不存在，会自动创建一个空的
    ctx.transport_manager
        .inject_connection(&dest, wire_handle)
        .await
        .map_err(|e| {
            JsValue::from_str(&format!(
                "Failed to inject connection for peer {}: {}",
                peer_id, e
            ))
        })?;

    log::info!(
        "[SW] register_datachannel_port: injected WebRTC connection for peer_id={}",
        peer_id
    );
    Ok(())
}

#[wasm_bindgen]
pub fn handle_dom_fast_path(client_id: String, payload: JsValue) -> Result<(), JsValue> {
    let data: FastPathPayload = serde_wasm_bindgen::from_value(payload)?;
    let runtime = CLIENTS.with(|cell| {
        cell.borrow()
            .get(&client_id)
            .map(|ctx| Rc::clone(&ctx.runtime))
    });
    if let Some(runtime) = runtime {
        wasm_bindgen_futures::spawn_local(async move {
            let mut rt = runtime.lock().await;
            let _ = rt.handle_fast_path(data);
        });
    } else {
        log::warn!(
            "[SW] handle_dom_fast_path: client not found client_id={}",
            client_id
        );
    }
    Ok(())
}
