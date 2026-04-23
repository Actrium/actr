//! Dynamic Workload implementation for callback interfaces
//!
//! The FFI-exposed workload API is shaped as a **multi-observer**:
//!
//! - [`WorkloadLifecycleBridge`] is mandatory: the foreign-language side
//!   supplies a single implementation that handles lifecycle hooks
//!   (`on_start` / `on_ready` / `on_stop` / `on_error`) plus `dispatch`
//!   (message routing).
//! - [`SignalingObserverBridge`], [`WebSocketObserverBridge`],
//!   [`WebRtcObserverBridge`], [`CredentialObserverBridge`] and
//!   [`MailboxObserverBridge`] are **optional** per-category observers.
//!   The foreign-language wrapper library may install only the observers
//!   it cares about; categories left as `None` fall back to the Rust
//!   framework's tracing defaults.
//!
//! This replaces the previous monolithic `WorkloadBridge` (lifecycle +
//! dispatch in one trait with no observation hooks).

use crate::ActrResult;
use crate::context::ContextBridge;
use actr_framework::{
    BackpressureEvent, Bytes, Context, CredentialEvent, ErrorCategory, ErrorEvent,
    MessageDispatcher, PeerEvent, Workload,
};
use actr_protocol::{ActorResult, RpcEnvelope};
use async_trait::async_trait;
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// FFI-safe records mirroring framework event types
// ─────────────────────────────────────────────────────────────────────────────

/// RPC Envelope exposed to FFI
///
/// Contains the route key, payload, and request ID for an RPC message.
#[derive(uniffi::Record, Clone)]
pub struct RpcEnvelopeBridge {
    /// Route key for the RPC method (e.g., "echo.EchoService.Echo")
    pub route_key: String,
    /// Request payload bytes (protobuf encoded)
    pub payload: Vec<u8>,
    /// Request ID for correlation
    pub request_id: String,
}

impl From<RpcEnvelope> for RpcEnvelopeBridge {
    fn from(envelope: RpcEnvelope) -> Self {
        Self {
            route_key: envelope.route_key,
            payload: envelope.payload.map(|p| p.to_vec()).unwrap_or_default(),
            request_id: envelope.request_id,
        }
    }
}

/// Peer-scoped event payload (WebSocket / WebRTC).
#[derive(uniffi::Record, Clone)]
pub struct PeerEventBridge {
    /// Remote peer identity.
    pub peer: crate::types::ActrId,
    /// `Some(true)` for WebRTC TURN-relayed, `Some(false)` for direct P2P,
    /// `None` for WebSocket (not applicable).
    pub relayed: Option<bool>,
}

impl From<&PeerEvent> for PeerEventBridge {
    fn from(ev: &PeerEvent) -> Self {
        Self {
            peer: ev.peer.clone().into(),
            relayed: ev.relayed,
        }
    }
}

/// Coarse error-event classification mirror of
/// [`actr_framework::ErrorCategory`].
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCategoryBridge {
    HandlerPanic,
    HandlerError,
    SignalingFailure,
    TransportFailure,
}

impl From<ErrorCategory> for ErrorCategoryBridge {
    fn from(c: ErrorCategory) -> Self {
        match c {
            ErrorCategory::HandlerPanic => ErrorCategoryBridge::HandlerPanic,
            ErrorCategory::HandlerError => ErrorCategoryBridge::HandlerError,
            ErrorCategory::SignalingFailure => ErrorCategoryBridge::SignalingFailure,
            ErrorCategory::TransportFailure => ErrorCategoryBridge::TransportFailure,
        }
    }
}

/// FFI-shaped error event.
///
/// `source` is the `Display` of the underlying [`actr_protocol::ActrError`]
/// (the enum itself cannot cross UniFFI unchanged), and `timestamp_ms` is
/// the wall-clock time encoded as milliseconds since the UNIX epoch.
#[derive(uniffi::Record, Clone)]
pub struct ErrorEventBridge {
    /// Stringified underlying error (see [`actr_protocol::ActrError`]).
    pub source: String,
    /// Error-domain classification.
    pub category: ErrorCategoryBridge,
    /// Free-form context (route key, handler name, stage).
    pub context: String,
    /// Wall-clock timestamp (milliseconds since UNIX epoch).
    pub timestamp_ms: i64,
}

impl From<&ErrorEvent> for ErrorEventBridge {
    fn from(ev: &ErrorEvent) -> Self {
        let timestamp_ms = ev
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self {
            source: ev.source.to_string(),
            category: ev.category.into(),
            context: ev.context.clone(),
            timestamp_ms,
        }
    }
}

/// Credential renewal / warning event.
#[derive(uniffi::Record, Clone)]
pub struct CredentialEventBridge {
    /// New credential expiry as milliseconds since UNIX epoch.
    pub new_expiry_ms: i64,
}

impl From<&CredentialEvent> for CredentialEventBridge {
    fn from(ev: &CredentialEvent) -> Self {
        let ms = ev
            .new_expiry
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self { new_expiry_ms: ms }
    }
}

/// Mailbox backpressure event.
#[derive(uniffi::Record, Clone, Copy)]
pub struct BackpressureEventBridge {
    pub queue_len: u64,
    pub threshold: u64,
}

impl From<&BackpressureEvent> for BackpressureEventBridge {
    fn from(ev: &BackpressureEvent) -> Self {
        Self {
            queue_len: ev.queue_len as u64,
            threshold: ev.threshold as u64,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Callback interfaces
// ─────────────────────────────────────────────────────────────────────────────

/// Required lifecycle + dispatch bridge.
///
/// The foreign-language code supplies exactly one implementation of this
/// interface. It handles the four fallible lifecycle hooks and the core
/// `dispatch` entry point.
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait WorkloadLifecycleBridge: Send + Sync + 'static {
    /// Called when the node has started.
    async fn on_start(&self, ctx: Arc<ContextBridge>) -> ActrResult<()>;

    /// Called when the node is ready to accept requests.
    async fn on_ready(&self, ctx: Arc<ContextBridge>) -> ActrResult<()>;

    /// Called when the node receives a shutdown signal.
    async fn on_stop(&self, ctx: Arc<ContextBridge>) -> ActrResult<()>;

    /// Called when the framework catches a runtime error.
    async fn on_error(&self, ctx: Arc<ContextBridge>, event: ErrorEventBridge) -> ActrResult<()>;

    /// Dispatch an incoming RPC message and return the response bytes.
    async fn dispatch(
        &self,
        ctx: Arc<ContextBridge>,
        envelope: RpcEnvelopeBridge,
    ) -> ActrResult<Vec<u8>>;
}

/// Optional observer for signaling-layer events.
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait SignalingObserverBridge: Send + Sync + 'static {
    async fn on_connecting(&self, ctx: Option<Arc<ContextBridge>>);
    async fn on_connected(&self, ctx: Option<Arc<ContextBridge>>);
    async fn on_disconnected(&self, ctx: Arc<ContextBridge>);
}

/// Optional observer for WebSocket peer events.
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait WebSocketObserverBridge: Send + Sync + 'static {
    async fn on_connecting(&self, ctx: Arc<ContextBridge>, event: PeerEventBridge);
    async fn on_connected(&self, ctx: Arc<ContextBridge>, event: PeerEventBridge);
    async fn on_disconnected(&self, ctx: Arc<ContextBridge>, event: PeerEventBridge);
}

/// Optional observer for WebRTC P2P peer events.
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait WebRtcObserverBridge: Send + Sync + 'static {
    async fn on_connecting(&self, ctx: Arc<ContextBridge>, event: PeerEventBridge);
    async fn on_connected(&self, ctx: Arc<ContextBridge>, event: PeerEventBridge);
    async fn on_disconnected(&self, ctx: Arc<ContextBridge>, event: PeerEventBridge);
}

/// Optional observer for credential lifecycle events.
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait CredentialObserverBridge: Send + Sync + 'static {
    async fn on_renewed(&self, ctx: Arc<ContextBridge>, event: CredentialEventBridge);
    async fn on_expiring(&self, ctx: Arc<ContextBridge>, event: CredentialEventBridge);
}

/// Optional observer for mailbox-backpressure events.
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait MailboxObserverBridge: Send + Sync + 'static {
    async fn on_backpressure(&self, ctx: Arc<ContextBridge>, event: BackpressureEventBridge);
}

// ─────────────────────────────────────────────────────────────────────────────
// DynamicWorkload — multi-observer wrapper
// ─────────────────────────────────────────────────────────────────────────────

/// Dynamic workload composed of one mandatory [`WorkloadLifecycleBridge`]
/// and up to five optional category observers.
///
/// Categories left as `None` fall back to the framework's built-in tracing
/// defaults when the hook fires.
#[derive(uniffi::Object)]
pub struct DynamicWorkload {
    lifecycle: Arc<dyn WorkloadLifecycleBridge>,
    #[allow(dead_code)]
    signaling: Option<Arc<dyn SignalingObserverBridge>>,
    #[allow(dead_code)]
    websocket: Option<Arc<dyn WebSocketObserverBridge>>,
    #[allow(dead_code)]
    webrtc: Option<Arc<dyn WebRtcObserverBridge>>,
    #[allow(dead_code)]
    credential: Option<Arc<dyn CredentialObserverBridge>>,
    #[allow(dead_code)]
    mailbox: Option<Arc<dyn MailboxObserverBridge>>,
}

#[uniffi::export]
impl DynamicWorkload {
    /// Construct a `DynamicWorkload` from a mandatory lifecycle bridge and
    /// a variadic set of optional per-category observers.
    #[uniffi::constructor]
    pub fn new(
        lifecycle: Box<dyn WorkloadLifecycleBridge>,
        signaling: Option<Box<dyn SignalingObserverBridge>>,
        websocket: Option<Box<dyn WebSocketObserverBridge>>,
        webrtc: Option<Box<dyn WebRtcObserverBridge>>,
        credential: Option<Box<dyn CredentialObserverBridge>>,
        mailbox: Option<Box<dyn MailboxObserverBridge>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            lifecycle: Arc::from(lifecycle),
            signaling: signaling.map(Arc::from),
            websocket: websocket.map(Arc::from),
            webrtc: webrtc.map(Arc::from),
            credential: credential.map(Arc::from),
            mailbox: mailbox.map(Arc::from),
        })
    }
}

#[async_trait]
impl Workload for DynamicWorkload {
    type Dispatcher = DynamicDispatcher;

    async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;
        self.lifecycle
            .on_start(ctx_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)
    }

    async fn on_ready<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;
        self.lifecycle
            .on_ready(ctx_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)
    }

    async fn on_stop<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;
        self.lifecycle
            .on_stop(ctx_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)
    }

    async fn on_error<C: Context>(&self, ctx: &C, event: &ErrorEvent) -> ActorResult<()> {
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;
        let ev_bridge: ErrorEventBridge = event.into();
        self.lifecycle
            .on_error(ctx_bridge, ev_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)
    }

    // ── Signaling observers ──────────────────────────────────────────────

    async fn on_signaling_connecting<C: Context>(&self, ctx: Option<&C>) {
        let Some(obs) = self.signaling.clone() else {
            return;
        };
        let ctx_bridge = ctx.and_then(|c| ContextBridge::try_from_context(c).ok());
        obs.on_connecting(ctx_bridge).await;
    }

    async fn on_signaling_connected<C: Context>(&self, ctx: Option<&C>) {
        let Some(obs) = self.signaling.clone() else {
            return;
        };
        let ctx_bridge = ctx.and_then(|c| ContextBridge::try_from_context(c).ok());
        obs.on_connected(ctx_bridge).await;
    }

    async fn on_signaling_disconnected<C: Context>(&self, ctx: &C) {
        let Some(obs) = self.signaling.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_disconnected(ctx_bridge).await;
    }

    // ── WebSocket observers ──────────────────────────────────────────────

    async fn on_websocket_connecting<C: Context>(&self, ctx: &C, event: &PeerEvent) {
        let Some(obs) = self.websocket.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_connecting(ctx_bridge, event.into()).await;
    }

    async fn on_websocket_connected<C: Context>(&self, ctx: &C, event: &PeerEvent) {
        let Some(obs) = self.websocket.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_connected(ctx_bridge, event.into()).await;
    }

    async fn on_websocket_disconnected<C: Context>(&self, ctx: &C, event: &PeerEvent) {
        let Some(obs) = self.websocket.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_disconnected(ctx_bridge, event.into()).await;
    }

    // ── WebRTC observers ─────────────────────────────────────────────────

    async fn on_webrtc_connecting<C: Context>(&self, ctx: &C, event: &PeerEvent) {
        let Some(obs) = self.webrtc.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_connecting(ctx_bridge, event.into()).await;
    }

    async fn on_webrtc_connected<C: Context>(&self, ctx: &C, event: &PeerEvent) {
        let Some(obs) = self.webrtc.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_connected(ctx_bridge, event.into()).await;
    }

    async fn on_webrtc_disconnected<C: Context>(&self, ctx: &C, event: &PeerEvent) {
        let Some(obs) = self.webrtc.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_disconnected(ctx_bridge, event.into()).await;
    }

    // ── Credential observers ─────────────────────────────────────────────

    async fn on_credential_renewed<C: Context>(&self, ctx: &C, event: &CredentialEvent) {
        let Some(obs) = self.credential.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_renewed(ctx_bridge, event.into()).await;
    }

    async fn on_credential_expiring<C: Context>(&self, ctx: &C, event: &CredentialEvent) {
        let Some(obs) = self.credential.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_expiring(ctx_bridge, event.into()).await;
    }

    // ── Mailbox observers ────────────────────────────────────────────────

    async fn on_mailbox_backpressure<C: Context>(&self, ctx: &C, event: &BackpressureEvent) {
        let Some(obs) = self.mailbox.clone() else {
            return;
        };
        let Ok(ctx_bridge) = ContextBridge::try_from_context(ctx) else {
            return;
        };
        obs.on_backpressure(ctx_bridge, event.into()).await;
    }
}

/// Dynamic dispatcher that routes messages to the callback interface.
///
/// All message-handling logic must live in the user's implementation of
/// [`WorkloadLifecycleBridge::dispatch`] on the foreign-language side.
pub struct DynamicDispatcher;

#[async_trait]
impl MessageDispatcher for DynamicDispatcher {
    type Workload = DynamicWorkload;

    async fn dispatch<C: Context>(
        workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;
        let envelope_bridge: RpcEnvelopeBridge = envelope.into();
        let response = workload
            .lifecycle
            .dispatch(ctx_bridge, envelope_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)?;
        Ok(Bytes::from(response))
    }
}
