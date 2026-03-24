//! Workload runtime abstractions for guest backends.
//!
//! This module replaces the old executor adapter layer. `ActrNode` dispatches
//! directly into a runtime `Workload` enum.

use actr_framework::guest::abi::{
    self as guest_abi, AbiPayload, GuestHandleV1, HostCallRawV1, HostCallV1, HostDiscoverV1,
    HostTellV1,
};
use actr_protocol::{Acl, ActorResult, ActrError, ActrId, RpcEnvelope};
use bytes::Bytes;
#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
use prost::Message;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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

/// Host-side trait object for native Rust workloads.
#[async_trait::async_trait]
pub trait NativeRuntimeWorkload: Debug + Send + Sync {
    /// Lifecycle hook triggered when the node is fully initialized.
    async fn on_start(&self, ctx: &crate::context::RuntimeContext) -> ActorResult<()>;

    /// Lifecycle hook triggered when the node is shutting down.
    async fn on_stop(&self, ctx: &crate::context::RuntimeContext) -> ActorResult<()>;

    /// Dispatch one inbound RPC envelope.
    async fn dispatch(
        &self,
        self_id: &ActrId,
        caller_id: Option<&ActrId>,
        envelope: RpcEnvelope,
        ctx: &crate::context::RuntimeContext,
    ) -> ActorResult<Bytes>;
}

/// Adapter that bridges a native Rust `Workload` into the runtime workload enum.
pub struct NativeWorkloadAdapter<W: actr_framework::Workload> {
    dispatch: actr_runtime::ActrDispatch<W>,
}

impl<W: actr_framework::Workload> NativeWorkloadAdapter<W> {
    pub fn new(workload: W, acl: Option<Acl>) -> Self {
        Self {
            dispatch: actr_runtime::ActrDispatch::new(Arc::new(workload), acl),
        }
    }
}

impl<W: actr_framework::Workload> Debug for NativeWorkloadAdapter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeWorkloadAdapter").finish()
    }
}

#[async_trait::async_trait]
impl<W> NativeRuntimeWorkload for NativeWorkloadAdapter<W>
where
    W: actr_framework::Workload + Send + Sync + 'static,
{
    async fn on_start(&self, ctx: &crate::context::RuntimeContext) -> ActorResult<()> {
        self.dispatch.on_start(ctx).await
    }

    async fn on_stop(&self, ctx: &crate::context::RuntimeContext) -> ActorResult<()> {
        self.dispatch.on_stop(ctx).await
    }

    async fn dispatch(
        &self,
        self_id: &ActrId,
        caller_id: Option<&ActrId>,
        envelope: RpcEnvelope,
        ctx: &crate::context::RuntimeContext,
    ) -> ActorResult<Bytes> {
        self.dispatch
            .dispatch(self_id, caller_id, envelope, ctx)
            .await
    }
}

/// Runtime workload enum.
#[derive(Debug)]
pub enum Workload {
    /// No guest workload attached. Inbound messages return an error immediately.
    /// Use this for pure client nodes that only make outbound calls.
    None,
    /// Native Rust workload attached directly to the runtime.
    Native(Box<dyn NativeRuntimeWorkload>),
    #[cfg(feature = "wasm-engine")]
    Wasm(crate::wasm::WasmWorkload),
    #[cfg(feature = "dynclib-engine")]
    DynClib(crate::dynclib::DynClibWorkload),
}

impl Workload {
    /// Build a runtime workload from a native Rust `Workload`.
    pub fn native<W>(workload: W, acl: Option<Acl>) -> Self
    where
        W: actr_framework::Workload + Send + Sync + 'static,
    {
        Self::Native(Box::new(NativeWorkloadAdapter::new(workload, acl)))
    }

    /// Run the workload start hook.
    pub fn on_start<'a>(
        &'a self,
        ctx: &'a crate::context::RuntimeContext,
    ) -> Pin<Box<dyn Future<Output = ActorResult<()>> + Send + 'a>> {
        Box::pin(async move {
            match self {
                Workload::None => Ok(()),
                Workload::Native(workload) => workload.on_start(ctx).await,
                #[cfg(feature = "wasm-engine")]
                Workload::Wasm(_) => Ok(()),
                #[cfg(feature = "dynclib-engine")]
                Workload::DynClib(_) => Ok(()),
            }
        })
    }

    /// Run the workload stop hook.
    pub fn on_stop<'a>(
        &'a self,
        ctx: &'a crate::context::RuntimeContext,
    ) -> Pin<Box<dyn Future<Output = ActorResult<()>> + Send + 'a>> {
        Box::pin(async move {
            match self {
                Workload::None => Ok(()),
                Workload::Native(workload) => workload.on_stop(ctx).await,
                #[cfg(feature = "wasm-engine")]
                Workload::Wasm(_) => Ok(()),
                #[cfg(feature = "dynclib-engine")]
                Workload::DynClib(_) => Ok(()),
            }
        })
    }

    /// Dispatch one inbound RPC envelope.
    pub fn dispatch_envelope<'a>(
        &'a mut self,
        envelope: RpcEnvelope,
        ctx: crate::context::RuntimeContext,
        invocation: InvocationContext,
        _host_abi: &'a HostAbiFn,
    ) -> Pin<Box<dyn Future<Output = ActorResult<Bytes>> + Send + 'a>> {
        Box::pin(async move {
            match self {
                Workload::None => Err(ActrError::Internal(
                    "no workload attached to this node".to_string(),
                )),
                Workload::Native(workload) => {
                    workload
                        .dispatch(
                            &invocation.self_id,
                            invocation.caller_id.as_ref(),
                            envelope,
                            &ctx,
                        )
                        .await
                }
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
                #[allow(unreachable_patterns)]
                _ => Err(ActrError::Internal(
                    "no workload backend is enabled in this build".to_string(),
                )),
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
            #[allow(unreachable_patterns)]
            match self {
                Workload::None => Err(Box::new(std::io::Error::other(
                    "no workload attached to this node",
                ))
                    as Box<dyn std::error::Error + Send + Sync>),
                Workload::Native(_) => Err(Box::new(std::io::Error::other(
                    "native workloads must be dispatched with an RpcEnvelope",
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
                _ => Err(Box::new(std::io::Error::other(
                    "no workload backend is enabled in this build",
                ))
                    as Box<dyn std::error::Error + Send + Sync>),
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
    let frame = request.into_frame()?;
    guest_abi::encode_message(&frame)
}

/// Decode guest-encoded [`DestV1`] back to [`actr_framework::Dest`].
///
/// Re-exported from `actr_framework::guest::abi` for host-side convenience.
pub fn decode_dest(v1: &actr_framework::guest::abi::DestV1) -> Option<actr_framework::Dest> {
    actr_framework::guest::abi::dest_v1_to_dest(v1)
}
