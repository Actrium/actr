//! Workload runtime abstractions for guest backends.
//!
//! This module replaces the old executor adapter layer. `ActrNode` dispatches
//! directly into a runtime `Workload` enum.

use actr_framework::guest::abi::{
    self as guest_abi, AbiPayload, GuestHandleV1, HostCallRawV1, HostCallV1, HostDiscoverV1,
    HostTellV1,
};
use actr_framework::{BackpressureEvent, CredentialEvent, ErrorEvent, PeerEvent};
use actr_protocol::{ActorResult, ActrError, RpcEnvelope};
use async_trait::async_trait;
use bytes::Bytes;
#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
use prost::Message;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::context::RuntimeContext;

/// ABI-stable invocation context passed into guest runtime on each request.
pub type InvocationContext = guest_abi::InvocationContextV1;

/// Guest-initiated host operation carrying strong-typed ABI payloads.
#[derive(Debug)]
pub enum HostOperation {
    Call(HostCallV1),
    Tell(HostTellV1),
    Discover(HostDiscoverV1),
    CallRaw(HostCallRawV1),
}

/// Result of a host operation.
#[derive(Debug)]
pub enum HostOperationResult {
    Bytes(Vec<u8>),
    Done,
    Error(i32),
}

/// Host-side async bridge used by guest runtimes.
pub type HostAbiFn = Box<
    dyn Fn(HostOperation) -> Pin<Box<dyn Future<Output = HostOperationResult> + Send>>
        + Send
        + Sync,
>;

/// Result type for runtime workload handling.
pub type WorkloadDispatchResult = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;

/// Object-safe handle to a workload linked directly into the host process
/// (e.g. an embedded Swift / Kotlin app, or a Rust process that owns the
/// actor's business code as a struct rather than a packaged binary).
///
/// Plugged into a [`crate::Node`] via
/// [`crate::Node::attach_linked_handle`]. The current implementation
/// registers the handle as a [`lifecycle::hooks::WorkloadHookObserver`] so
/// that signaling / transport / credential / mailbox lifecycle hooks reach
/// the embedding app. Inbound RPC dispatch into a linked handle is not
/// yet wired — linked-workload nodes are client-only until a future change
/// adds a dispatch path that cooperates with the generic
/// [`actr_framework::Workload`] trait.
#[async_trait]
pub trait LinkedWorkloadHandle: Send + Sync + 'static {
    // Lifecycle (fallible — hook-path errors are logged & swallowed)
    async fn on_start(&self, _ctx: &RuntimeContext) {}
    async fn on_ready(&self, _ctx: &RuntimeContext) {}
    async fn on_stop(&self, _ctx: &RuntimeContext) {}
    async fn on_error(&self, _ctx: &RuntimeContext, _event: &ErrorEvent) {}

    // Signaling
    async fn on_signaling_connecting(&self, _ctx: Option<&RuntimeContext>) {}
    async fn on_signaling_connected(&self, _ctx: Option<&RuntimeContext>) {}
    async fn on_signaling_disconnected(&self, _ctx: &RuntimeContext) {}

    // WebSocket C/S
    async fn on_websocket_connecting(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_websocket_connected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_websocket_disconnected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}

    // WebRTC P2P
    async fn on_webrtc_connecting(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_webrtc_connected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_webrtc_disconnected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}

    // Credential
    async fn on_credential_renewed(&self, _ctx: &RuntimeContext, _event: &CredentialEvent) {}
    async fn on_credential_expiring(&self, _ctx: &RuntimeContext, _event: &CredentialEvent) {}

    // Mailbox
    async fn on_mailbox_backpressure(
        &self,
        _ctx: &RuntimeContext,
        _event: &BackpressureEvent,
    ) {
    }
}

/// Bridge adapter: forwards every [`LinkedWorkloadHandle`] method to the
/// `pub(crate)` [`crate::lifecycle::hooks::WorkloadHookObserver`] expected by
/// the hook dispatcher. Lets the public linked-handle trait live without
/// exposing the internal hook plumbing.
pub(crate) struct LinkedHandleObserver {
    pub(crate) handle: Arc<dyn LinkedWorkloadHandle>,
}

#[async_trait]
impl crate::lifecycle::hooks::WorkloadHookObserver for LinkedHandleObserver {
    async fn on_start(&self, ctx: &RuntimeContext) {
        self.handle.on_start(ctx).await
    }
    async fn on_ready(&self, ctx: &RuntimeContext) {
        self.handle.on_ready(ctx).await
    }
    async fn on_stop(&self, ctx: &RuntimeContext) {
        self.handle.on_stop(ctx).await
    }
    async fn on_error(&self, ctx: &RuntimeContext, event: &ErrorEvent) {
        self.handle.on_error(ctx, event).await
    }
    async fn on_signaling_connecting(&self, ctx: Option<&RuntimeContext>) {
        self.handle.on_signaling_connecting(ctx).await
    }
    async fn on_signaling_connected(&self, ctx: Option<&RuntimeContext>) {
        self.handle.on_signaling_connected(ctx).await
    }
    async fn on_signaling_disconnected(&self, ctx: &RuntimeContext) {
        self.handle.on_signaling_disconnected(ctx).await
    }
    async fn on_websocket_connecting(&self, ctx: &RuntimeContext, event: &PeerEvent) {
        self.handle.on_websocket_connecting(ctx, event).await
    }
    async fn on_websocket_connected(&self, ctx: &RuntimeContext, event: &PeerEvent) {
        self.handle.on_websocket_connected(ctx, event).await
    }
    async fn on_websocket_disconnected(&self, ctx: &RuntimeContext, event: &PeerEvent) {
        self.handle.on_websocket_disconnected(ctx, event).await
    }
    async fn on_webrtc_connecting(&self, ctx: &RuntimeContext, event: &PeerEvent) {
        self.handle.on_webrtc_connecting(ctx, event).await
    }
    async fn on_webrtc_connected(&self, ctx: &RuntimeContext, event: &PeerEvent) {
        self.handle.on_webrtc_connected(ctx, event).await
    }
    async fn on_webrtc_disconnected(&self, ctx: &RuntimeContext, event: &PeerEvent) {
        self.handle.on_webrtc_disconnected(ctx, event).await
    }
    async fn on_credential_renewed(&self, ctx: &RuntimeContext, event: &CredentialEvent) {
        self.handle.on_credential_renewed(ctx, event).await
    }
    async fn on_credential_expiring(&self, ctx: &RuntimeContext, event: &CredentialEvent) {
        self.handle.on_credential_expiring(ctx, event).await
    }
    async fn on_mailbox_backpressure(
        &self,
        ctx: &RuntimeContext,
        event: &BackpressureEvent,
    ) {
        self.handle.on_mailbox_backpressure(ctx, event).await
    }
}

/// Runtime workload enum.
///
/// Covers three attach flavours:
///
/// - `Wasm` / `DynClib` — a verified `.actr` package bound through
///   [`crate::Node::attach`]. The package carries a guest binary that the
///   host dispatches RPC envelopes into.
/// - `None` — client-only attach via [`crate::Node::attach_none`] or a
///   linked-workload attach without a dispatchable guest. Inbound RPC
///   dispatch is not supported on this variant; the node is strictly for
///   outbound calls / discovery.
#[derive(Debug, Default)]
#[allow(clippy::large_enum_variant)]
pub enum Workload {
    /// Client-only node: no local guest, no dispatch target.
    #[default]
    None,
    #[cfg(feature = "wasm-engine")]
    Wasm(crate::wasm::WasmWorkload),
    #[cfg(feature = "dynclib-engine")]
    DynClib(crate::dynclib::DynClibWorkload),
}

impl Workload {
    /// Dispatch one inbound RPC envelope.
    pub fn dispatch_envelope<'a>(
        &'a mut self,
        envelope: RpcEnvelope,
        _ctx: crate::context::RuntimeContext,
        invocation: InvocationContext,
        _host_abi: &'a HostAbiFn,
    ) -> Pin<Box<dyn Future<Output = ActorResult<Bytes>> + Send + 'a>> {
        Box::pin(async move {
            let _ = (&envelope, &invocation);
            match self {
                Workload::None => Err(ActrError::NotImplemented(
                    "this node has no dispatchable workload (client-only attach)".to_string(),
                )),
                #[cfg(feature = "wasm-engine")]
                Workload::Wasm(workload) => {
                    let request_bytes = envelope.encode_to_vec();
                    workload
                        .handle(&request_bytes, invocation, _host_abi)
                        .await
                        .map(Bytes::from)
                        .map_err(|e| ActrError::Internal(format!("workload dispatch failed: {e}")))
                }
                #[cfg(feature = "dynclib-engine")]
                Workload::DynClib(workload) => {
                    let request_bytes = envelope.encode_to_vec();
                    workload
                        .handle(&request_bytes, invocation, _host_abi)
                        .await
                        .map(Bytes::from)
                        .map_err(|e| ActrError::Internal(format!("workload dispatch failed: {e}")))
                }
            }
        })
    }

    /// Handle one incoming request through the selected backend.
    #[allow(unused_variables)]
    pub fn handle<'a>(
        &'a mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        host_abi: &'a HostAbiFn,
    ) -> Pin<Box<dyn Future<Output = WorkloadDispatchResult> + Send + 'a>> {
        let request_bytes = request_bytes.to_vec();
        Box::pin(async move {
            match self {
                Workload::None => Err(Box::new(std::io::Error::other(
                    "this node has no dispatchable workload (client-only attach)",
                ))
                    as Box<dyn std::error::Error + Send + Sync>),
                #[cfg(feature = "wasm-engine")]
                Workload::Wasm(workload) => workload
                    .handle(&request_bytes, ctx, host_abi)
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
                #[cfg(feature = "dynclib-engine")]
                Workload::DynClib(workload) => workload
                    .handle(&request_bytes, ctx, host_abi)
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
            }
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared host-side helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Decode an [`guest_abi::AbiFrame`] into a strongly-typed [`HostOperation`].
///
/// Shared by both WASM and DynClib host backends.
pub fn decode_host_operation(frame: guest_abi::AbiFrame) -> Result<HostOperation, i32> {
    if frame.abi_version != guest_abi::version::V1 {
        return Err(guest_abi::code::PROTOCOL_ERROR);
    }

    match frame.op {
        guest_abi::op::HOST_CALL => {
            let payload = <HostCallV1 as AbiPayload>::decode_payload(&frame.payload)?;
            Ok(HostOperation::Call(payload))
        }
        guest_abi::op::HOST_TELL => {
            let payload = <HostTellV1 as AbiPayload>::decode_payload(&frame.payload)?;
            Ok(HostOperation::Tell(payload))
        }
        guest_abi::op::HOST_CALL_RAW => {
            let payload = <HostCallRawV1 as AbiPayload>::decode_payload(&frame.payload)?;
            Ok(HostOperation::CallRaw(payload))
        }
        guest_abi::op::HOST_DISCOVER => {
            let payload = <HostDiscoverV1 as AbiPayload>::decode_payload(&frame.payload)?;
            Ok(HostOperation::Discover(payload))
        }
        _ => Err(guest_abi::code::UNSUPPORTED_OP),
    }
}

/// Encode an inbound guest dispatch as `GuestHandleV1` wrapped in `AbiFrame`.
pub fn encode_guest_handle_request(
    request_bytes: &[u8],
    ctx: InvocationContext,
) -> Result<Vec<u8>, i32> {
    let request = GuestHandleV1 {
        ctx,
        rpc_envelope: request_bytes.to_vec(),
    };
    let frame = request.to_frame()?;
    guest_abi::encode_message(&frame)
}

/// Decode guest-encoded [`DestV1`] back to [`actr_framework::Dest`].
///
/// Re-exported from `actr_framework::guest::abi` for host-side convenience.
pub fn decode_dest(v1: &actr_framework::guest::abi::DestV1) -> Option<actr_framework::Dest> {
    actr_framework::guest::abi::dest_v1_to_dest(v1)
}
