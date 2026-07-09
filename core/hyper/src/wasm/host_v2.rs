//! WasmWorkloadV2 — the `actr:workload@0.2.0` async-world execution path.
//!
//! Sibling of [`super::host::WasmWorkload`] (the 0.1.0 serial path). Where
//! the V1 path drives the guest through `call_dispatch(&mut store, ...)`
//! (borrow-checker-serialized), V2 drives it through a
//! `Store::run_concurrent(async |accessor| ...)` region and Accessor-based
//! host imports. Under M4 the region holds exactly ONE task at a time (the
//! runner is still serial), so behaviour is identical to V1 end-to-end;
//! M5 opens the region to `FuturesUnordered` for real same-instance
//! concurrency with zero further changes to the host-import side (each
//! in-flight invocation keys its `HostAbiFn` by `ctx-token`).
//!
//! The host-import trait here is Accessor-based: methods are static async
//! associated functions taking `&Accessor<HostState, Self>`, and store
//! access is synchronous-only via `accessor.with(|a| ...)` (its borrow
//! cannot cross an `.await`). This is what makes several `&mut Store`
//! borrows non-overlapping across suspension points. The shape is lifted
//! directly from the Phase 0.75 `component-spike-runconcurrent` host.

use actr_framework::guest::dynclib_abi as guest_abi;
use actr_framework::guest::dynclib_abi::{
    HostCallRawV1, HostCallV1, HostDiscoverV1, HostRegisterStreamV1, HostSendDataChunkV1,
    HostTellV1, HostUnregisterStreamV1,
};
use actr_framework::{BackpressureEvent, CredentialEvent, PeerEvent, WebRtcPeerStatus};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    ActrError, ActrId, ActrType, ConnectionNotReadyInfo, DataChunk, MetadataEntry, PayloadType,
    Realm, RpcEnvelope,
};
use wasmtime::component::{Accessor, Component, HasData, Linker};
use wasmtime::{Engine, Store};

use super::component_bindings_v2::ActrWorkloadGuestV2;
use super::component_bindings_v2::actr::workload::host::{Host as HostImportsV2, HostWithStore};
use super::component_bindings_v2::actr::workload::types::{
    self as wit2, ActrError as WitActrError, ActrId as WitActrId, ActrType as WitActrType,
    BackpressureEvent as WitBackpressureEvent, CredentialEvent as WitCredentialEvent,
    DataChunk as WitDataChunk, Dest as WitDest, Host as TypesHostV2,
    InvocationCtx as WitInvocationCtx, PayloadType as WitPayloadType, PeerEvent as WitPeerEvent,
    Realm as WitRealm, RpcEnvelope as WitRpcEnvelope, WebrtcPeerStatus as WitWebrtcPeerStatus,
};
use super::error::{WasmError, WasmResult};
use super::host::HostState;
use crate::executor::{ActorCmd, LifecyclePhase};
use crate::workload::{
    HostAbiFn, HostOperation, HostOperationResult, InvocationContext, PackageHookEvent,
};
use actr_protocol::ActorResult;
use bytes::Bytes;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use futures_util::stream::{FuturesUnordered, StreamExt as _};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::Instrument as _;

// ─────────────────────────────────────────────────────────────────────────────
// HasData projection + host-import Accessor trait
// ─────────────────────────────────────────────────────────────────────────────

// `type Data<'a> = &'a mut HostState` means "give the Accessor host methods
// `&mut HostState`". Required by the wasmtime 46 async binding shape.
impl HasData for HostState {
    type Data<'a> = &'a mut HostState;
}

// `types` is a types-only interface; bindgen still emits a marker `Host`
// trait the host state must implement. Empty impl satisfies the linker.
impl TypesHostV2 for HostState {}

// Store-less marker trait (imports needing only `self`). The blanket
// `impl Host for &mut T` needs this to exist; empty impl suffices.
impl HostImportsV2 for HostState {}

/// Forward a guest-initiated [`HostOperation`] through a `HostAbiFn` cloned
/// out of the per-invocation table (keyed by `ctx-token`). The `HostAbiFn`
/// is an `Arc`, cloned synchronously via `accessor.with` before this future
/// is awaited, so no store borrow is held across the `.await`.
async fn run_host_operation(
    host_abi: Option<HostAbiFn>,
    op: HostOperation,
) -> wasmtime::Result<Result<Vec<u8>, WitActrError>> {
    let Some(host_abi) = host_abi else {
        return Err(wasmtime::Error::msg(
            "host ABI bridge not installed for this ctx-token (unknown or retired invocation)",
        ));
    };
    match (host_abi)(op).await {
        HostOperationResult::Bytes(bytes) => Ok(Ok(bytes)),
        HostOperationResult::Done => Ok(Ok(Vec::new())),
        HostOperationResult::Error(code) => Ok(Err(actr_error_from_abi_code(code))),
    }
}

impl HostWithStore<HostState> for HostState {
    async fn call(
        accessor: &Accessor<HostState, Self>,
        ctx_token: u64,
        target: WitDest,
        route_key: String,
        payload: Vec<u8>,
    ) -> wasmtime::Result<Result<Vec<u8>, WitActrError>> {
        let host_abi = accessor.with(|mut a| a.get().invocation_host_abi(ctx_token));
        let op = HostOperation::Call(HostCallV1 {
            route_key,
            dest: wit_dest_to_v1(&target),
            payload,
        });
        run_host_operation(host_abi, op).await
    }

    async fn tell(
        accessor: &Accessor<HostState, Self>,
        ctx_token: u64,
        target: WitDest,
        route_key: String,
        payload: Vec<u8>,
    ) -> wasmtime::Result<Result<(), WitActrError>> {
        let host_abi = accessor.with(|mut a| a.get().invocation_host_abi(ctx_token));
        let op = HostOperation::Tell(HostTellV1 {
            route_key,
            dest: wit_dest_to_v1(&target),
            payload,
        });
        match run_host_operation(host_abi, op).await? {
            Ok(_) => Ok(Ok(())),
            Err(e) => Ok(Err(e)),
        }
    }

    async fn call_raw(
        accessor: &Accessor<HostState, Self>,
        ctx_token: u64,
        target: WitActrId,
        route_key: String,
        payload: Vec<u8>,
    ) -> wasmtime::Result<Result<Vec<u8>, WitActrError>> {
        let host_abi = accessor.with(|mut a| a.get().invocation_host_abi(ctx_token));
        let op = HostOperation::CallRaw(HostCallRawV1 {
            route_key,
            target: wit_actr_id_to_proto(&target),
            payload,
        });
        run_host_operation(host_abi, op).await
    }

    async fn discover(
        accessor: &Accessor<HostState, Self>,
        ctx_token: u64,
        target_type: WitActrType,
    ) -> wasmtime::Result<Result<WitActrId, WitActrError>> {
        let host_abi = accessor.with(|mut a| a.get().invocation_host_abi(ctx_token));
        let op = HostOperation::Discover(HostDiscoverV1 {
            target_type: wit_actr_type_to_proto(&target_type),
        });
        match run_host_operation(host_abi, op).await? {
            Ok(bytes) => match ActrId::decode(bytes.as_slice()) {
                Ok(id) => Ok(Ok(proto_actr_id_to_wit(&id))),
                Err(e) => Ok(Err(WitActrError::DecodeFailure(format!(
                    "host discover returned undecodable ActrId: {e}"
                )))),
            },
            Err(e) => Ok(Err(e)),
        }
    }

    async fn register_stream(
        accessor: &Accessor<HostState, Self>,
        ctx_token: u64,
        stream_id: String,
    ) -> wasmtime::Result<Result<(), WitActrError>> {
        let host_abi = accessor.with(|mut a| a.get().invocation_host_abi(ctx_token));
        let op = HostOperation::RegisterStream(HostRegisterStreamV1 { stream_id });
        match run_host_operation(host_abi, op).await? {
            Ok(_) => Ok(Ok(())),
            Err(e) => Ok(Err(e)),
        }
    }

    async fn unregister_stream(
        accessor: &Accessor<HostState, Self>,
        ctx_token: u64,
        stream_id: String,
    ) -> wasmtime::Result<Result<(), WitActrError>> {
        let host_abi = accessor.with(|mut a| a.get().invocation_host_abi(ctx_token));
        let op = HostOperation::UnregisterStream(HostUnregisterStreamV1 { stream_id });
        match run_host_operation(host_abi, op).await? {
            Ok(_) => Ok(Ok(())),
            Err(e) => Ok(Err(e)),
        }
    }

    async fn send_data_chunk(
        accessor: &Accessor<HostState, Self>,
        ctx_token: u64,
        target: WitDest,
        chunk: WitDataChunk,
        payload_type: WitPayloadType,
    ) -> wasmtime::Result<Result<(), WitActrError>> {
        let host_abi = accessor.with(|mut a| a.get().invocation_host_abi(ctx_token));
        let op = HostOperation::SendDataChunk(HostSendDataChunkV1 {
            dest: wit_dest_to_v1(&target),
            chunk: wit_data_chunk_to_proto(chunk),
            payload_type: wit_payload_type_to_proto(payload_type) as i32,
        });
        match run_host_operation(host_abi, op).await? {
            Ok(_) => Ok(Ok(())),
            Err(e) => Ok(Err(e)),
        }
    }

    async fn log_message(
        _accessor: &Accessor<HostState, Self>,
        _ctx_token: u64,
        level: String,
        message: String,
    ) -> wasmtime::Result<()> {
        match level.as_str() {
            "error" => tracing::error!(target: "wasm-guest", "{message}"),
            "warn" => tracing::warn!(target: "wasm-guest", "{message}"),
            "info" => tracing::info!(target: "wasm-guest", "{message}"),
            "debug" => tracing::debug!(target: "wasm-guest", "{message}"),
            "trace" => tracing::trace!(target: "wasm-guest", "{message}"),
            other => tracing::info!(target: "wasm-guest", level = %other, "{message}"),
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WIT (0.2.0) ↔ actr_protocol / actr_framework translation
//
// The 0.2.0 bindgen emits its own distinct type namespace, so these mirror
// the 0.1.0 helpers in `host.rs` but target the v2 `wit2` structs.
// ─────────────────────────────────────────────────────────────────────────────

fn wit_realm_to_proto(r: &WitRealm) -> Realm {
    Realm {
        realm_id: r.realm_id,
    }
}

fn proto_realm_to_wit(r: &Realm) -> WitRealm {
    WitRealm {
        realm_id: r.realm_id,
    }
}

fn wit_actr_type_to_proto(t: &WitActrType) -> ActrType {
    ActrType {
        manufacturer: t.manufacturer.clone(),
        name: t.name.clone(),
        version: t.version.clone(),
    }
}

fn proto_actr_type_to_wit(t: &ActrType) -> WitActrType {
    WitActrType {
        manufacturer: t.manufacturer.clone(),
        name: t.name.clone(),
        version: t.version.clone(),
    }
}

fn wit_actr_id_to_proto(id: &WitActrId) -> ActrId {
    ActrId {
        realm: wit_realm_to_proto(&id.realm),
        serial_number: id.serial_number,
        r#type: wit_actr_type_to_proto(&id.type_),
    }
}

fn proto_actr_id_to_wit(id: &ActrId) -> WitActrId {
    WitActrId {
        realm: proto_realm_to_wit(&id.realm),
        serial_number: id.serial_number,
        type_: proto_actr_type_to_wit(&id.r#type),
    }
}

fn wit_connection_not_ready_info_to_proto(
    info: wit2::ConnectionNotReadyInfo,
) -> ConnectionNotReadyInfo {
    ConnectionNotReadyInfo {
        retry_after_ms: info.retry_after_ms,
    }
}

fn wit_dest_to_v1(dest: &WitDest) -> guest_abi::DestV1 {
    match dest {
        WitDest::Shell => guest_abi::DestV1::shell(),
        WitDest::Local => guest_abi::DestV1::local(),
        WitDest::Actor(id) => guest_abi::DestV1::actor(wit_actr_id_to_proto(id)),
    }
}

fn actr_error_from_abi_code(code: i32) -> WitActrError {
    match code {
        guest_abi::code::GENERIC_ERROR => WitActrError::Internal("generic ABI error".into()),
        guest_abi::code::INIT_FAILED => WitActrError::Internal("init failed".into()),
        guest_abi::code::HANDLE_FAILED => WitActrError::Internal("handle failed".into()),
        guest_abi::code::ALLOC_FAILED => WitActrError::Internal("allocation failed".into()),
        guest_abi::code::PROTOCOL_ERROR => WitActrError::DecodeFailure("protocol error".into()),
        guest_abi::code::BUFFER_TOO_SMALL => {
            WitActrError::Internal("reply buffer too small".into())
        }
        guest_abi::code::UNSUPPORTED_OP => {
            WitActrError::NotImplemented("unsupported ABI operation".into())
        }
        other => WitActrError::Internal(format!("ABI status {other}")),
    }
}

fn wit_actr_error_to_proto(e: WitActrError) -> ActrError {
    match e {
        WitActrError::Unavailable(msg) => ActrError::Unavailable(msg),
        WitActrError::ConnectionNotReady(info) => {
            ActrError::ConnectionNotReady(wit_connection_not_ready_info_to_proto(info))
        }
        WitActrError::TimedOut => ActrError::TimedOut,
        WitActrError::NotFound(msg) => ActrError::NotFound(msg),
        WitActrError::PermissionDenied(msg) => ActrError::PermissionDenied(msg),
        WitActrError::InvalidArgument(msg) => ActrError::InvalidArgument(msg),
        WitActrError::UnknownRoute(msg) => ActrError::UnknownRoute(msg),
        WitActrError::DependencyNotFound(p) => ActrError::DependencyNotFound {
            service_name: p.service_name,
            message: p.message,
        },
        WitActrError::DecodeFailure(msg) => ActrError::DecodeFailure(msg),
        WitActrError::NotImplemented(msg) => ActrError::NotImplemented(msg),
        WitActrError::Internal(msg) => ActrError::Internal(msg),
    }
}

fn rpc_envelope_to_wit(envelope: &RpcEnvelope) -> WitRpcEnvelope {
    WitRpcEnvelope {
        request_id: envelope.request_id.clone(),
        route_key: envelope.route_key.clone(),
        payload: envelope
            .payload
            .as_ref()
            .map(|b| b.to_vec())
            .unwrap_or_default(),
    }
}

fn invocation_ctx_to_wit(ctx: &InvocationContext, ctx_token: u64) -> WitInvocationCtx {
    WitInvocationCtx {
        ctx_token,
        self_id: proto_actr_id_to_wit(&ctx.self_id),
        caller_id: ctx.caller_id.as_ref().map(proto_actr_id_to_wit),
        request_id: ctx.request_id.clone(),
    }
}

fn proto_data_chunk_to_wit(chunk: DataChunk) -> WitDataChunk {
    WitDataChunk {
        stream_id: chunk.stream_id,
        sequence: chunk.sequence,
        payload: chunk.payload.to_vec(),
        metadata: chunk
            .metadata
            .into_iter()
            .map(|entry| wit2::MetadataEntry {
                key: entry.key,
                value: entry.value,
            })
            .collect(),
        timestamp_ms: chunk.timestamp_ms,
    }
}

fn proto_peer_event_to_wit(event: PeerEvent) -> WitPeerEvent {
    WitPeerEvent {
        peer: proto_actr_id_to_wit(&event.peer),
        relayed: event.relayed,
        status: event.status.map(proto_webrtc_peer_status_to_wit),
    }
}

fn proto_webrtc_peer_status_to_wit(status: WebRtcPeerStatus) -> WitWebrtcPeerStatus {
    match status {
        WebRtcPeerStatus::Idle => WitWebrtcPeerStatus::Idle,
        WebRtcPeerStatus::Connecting => WitWebrtcPeerStatus::Connecting,
        WebRtcPeerStatus::Connected => WitWebrtcPeerStatus::Connected,
        WebRtcPeerStatus::Recovering => WitWebrtcPeerStatus::Recovering,
    }
}

fn system_time_to_wit(time: std::time::SystemTime) -> wit2::Timestamp {
    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    wit2::Timestamp {
        seconds: duration.as_secs(),
        nanoseconds: duration.subsec_nanos(),
    }
}

fn proto_credential_event_to_wit(event: CredentialEvent) -> WitCredentialEvent {
    WitCredentialEvent {
        new_expiry: system_time_to_wit(event.new_expiry),
    }
}

fn proto_backpressure_event_to_wit(event: BackpressureEvent) -> WitBackpressureEvent {
    WitBackpressureEvent {
        queue_len: event.queue_len as u64,
        threshold: event.threshold as u64,
    }
}

fn wit_data_chunk_to_proto(chunk: WitDataChunk) -> DataChunk {
    DataChunk {
        stream_id: chunk.stream_id,
        sequence: chunk.sequence,
        payload: chunk.payload.into(),
        metadata: chunk
            .metadata
            .into_iter()
            .map(|entry| MetadataEntry {
                key: entry.key,
                value: entry.value,
            })
            .collect(),
        timestamp_ms: chunk.timestamp_ms,
    }
}

fn wit_payload_type_to_proto(payload_type: WitPayloadType) -> PayloadType {
    match payload_type {
        WitPayloadType::RpcReliable => PayloadType::RpcReliable,
        WitPayloadType::RpcSignal => PayloadType::RpcSignal,
        WitPayloadType::StreamReliable => PayloadType::StreamReliable,
        WitPayloadType::StreamLatencyFirst => PayloadType::StreamLatencyFirst,
        WitPayloadType::MediaRtp => PayloadType::MediaRtp,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Instantiation
// ─────────────────────────────────────────────────────────────────────────────

/// Build a fresh async-world [`Store`] + component instance. Registers WASI
/// p2 and the Accessor-based `actr:workload/host` (0.2.0) linker imports.
async fn instantiate_parts_v2(
    engine: &Engine,
    component: &Component,
) -> WasmResult<(Store<HostState>, ActrWorkloadGuestV2)> {
    let mut linker: Linker<HostState> = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|e| {
        WasmError::LoadFailed(format!("failed to register WASI p2 linker imports: {e}"))
    })?;
    // D = HostState (impls HasData + HostWithStore + Host); host_getter is identity.
    super::component_bindings_v2::actr::workload::host::add_to_linker::<HostState, HostState>(
        &mut linker,
        |s| s,
    )
    .map_err(|e| {
        WasmError::LoadFailed(format!(
            "failed to register actr:workload/host@0.2.0 linker imports: {e}"
        ))
    })?;

    let mut store = Store::new(engine, HostState::new());
    let bindings = ActrWorkloadGuestV2::instantiate_async(&mut store, component, &linker)
        .await
        .map_err(|e| {
            WasmError::LoadFailed(format!("Component instantiate_async (v2) failed: {e:#}"))
        })?;
    Ok((store, bindings))
}

// ─────────────────────────────────────────────────────────────────────────────
// WasmWorkloadV2
// ─────────────────────────────────────────────────────────────────────────────

/// Single 0.2.0 async-world wasm actor instance.
///
/// Mirrors [`super::host::WasmWorkload`]'s lifecycle (engine/component/store
/// plus poison/rebuild), but every guest entry runs inside a single-task
/// `Store::run_concurrent` region. The per-invocation `ctx-token` is
/// allocated into [`HostState`]'s invocation table just before the region
/// opens and retired after it closes; a trap clears the whole table.
pub(crate) struct WasmWorkloadV2 {
    engine: Engine,
    component: Component,
    store: Store<HostState>,
    bindings: ActrWorkloadGuestV2,
    poisoned: bool,
    rebuilds: u64,
}

impl std::fmt::Debug for WasmWorkloadV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmWorkloadV2")
            .field("poisoned", &self.poisoned)
            .field("rebuilds", &self.rebuilds)
            .finish_non_exhaustive()
    }
}

impl WasmWorkloadV2 {
    /// Build a V2 instance from an already-compiled engine/component pair.
    pub(crate) async fn instantiate(engine: &Engine, component: &Component) -> WasmResult<Self> {
        let (store, bindings) = instantiate_parts_v2(engine, component).await?;
        tracing::info!("wasm Component instantiated (v2 async world)");
        Ok(Self {
            engine: engine.clone(),
            component: component.clone(),
            store,
            bindings,
            poisoned: false,
            rebuilds: 0,
        })
    }

    /// Legacy init entry — mirrors the V1 path so the loader stays uniform.
    pub(crate) fn init(&mut self, init_payload: &guest_abi::InitPayloadV1) -> WasmResult<()> {
        tracing::debug!(
            actr_type = %init_payload.actr_type,
            realm_id = init_payload.realm_id,
            "wasm Component workload init (v2; Component-model lifecycle handles this implicitly)"
        );
        Ok(())
    }

    pub(crate) fn rebuild_count(&self) -> u64 {
        self.rebuilds
    }

    /// Rebuild a poisoned store (fresh Store + re-instantiate), discarding
    /// the guest's in-memory state. No-op if not poisoned.
    async fn ensure_instance(&mut self) -> WasmResult<()> {
        if !self.poisoned {
            return Ok(());
        }
        tracing::warn!(
            rebuild_attempt = self.rebuilds + 1,
            "rebuilding poisoned wasm store (v2) after a prior guest trap; \
             guest in-memory state is discarded (lifecycle/queue not replayed)"
        );
        match instantiate_parts_v2(&self.engine, &self.component).await {
            Ok((store, bindings)) => {
                self.store = store;
                self.bindings = bindings;
                self.poisoned = false;
                self.rebuilds += 1;
                tracing::info!(
                    rebuilds = self.rebuilds,
                    "wasm store rebuilt (v2); serviceable"
                );
                Ok(())
            }
            Err(e) => Err(WasmError::LoadFailed(format!(
                "failed to rebuild poisoned wasm store (v2): {e}"
            ))),
        }
    }

    /// Mark the store poisoned after a trap and clear the whole invocation
    /// table (every in-flight token is dead). Returns a distinct
    /// [`WasmError::InstanceTrapped`].
    fn trap_poison(&mut self, entry: &str, trap: wasmtime::Error) -> WasmError {
        self.poisoned = true;
        self.store.data_mut().clear_invocations();
        tracing::error!(
            entry,
            error = %trap,
            "wasm guest trapped (v2); store poisoned (instance-level fatal). \
             In-memory guest state is lost; a fresh instance is rebuilt before the next call"
        );
        WasmError::InstanceTrapped(format!("{entry} trap: {trap}"))
    }

    /// Handle one inbound RPC request through the async world.
    pub(crate) async fn handle(
        &mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> WasmResult<Vec<u8>> {
        self.ensure_instance().await?;

        let envelope = RpcEnvelope::decode(request_bytes).map_err(|e| {
            WasmError::ExecutionFailed(format!(
                "host failed to decode RpcEnvelope before dispatch: {e}"
            ))
        })?;
        let wit_envelope = rpc_envelope_to_wit(&envelope);

        // Register this invocation and thread its token into the guest.
        let token = self
            .store
            .data_mut()
            .alloc_invocation(ctx.clone(), host_abi.clone());
        let inv = invocation_ctx_to_wit(&ctx, token);

        let bindings = &self.bindings;
        let region = self
            .store
            .run_concurrent(async move |accessor| {
                bindings
                    .actr_workload_workload()
                    .call_dispatch(accessor, wit_envelope, inv)
                    .await
            })
            .await;

        // Region closed: retire the token (unless the whole table was
        // cleared by a trap-poison below).
        if !self.poisoned {
            self.store.data_mut().remove_invocation(token);
        }

        match region {
            // Region-level failure (trap surfaced out of run_concurrent).
            Err(trap) => Err(self.trap_poison("dispatch", trap)),
            Ok(call) => match call {
                Ok(Ok(bytes)) => Ok(bytes),
                Ok(Err(wit_err)) => Err(WasmError::ExecutionFailed(format!(
                    "guest dispatch returned error: {:?}",
                    wit_actr_error_to_proto(wit_err)
                ))),
                Err(trap) => Err(self.trap_poison("dispatch", trap)),
            },
        }
    }

    pub(crate) async fn call_on_start(
        &mut self,
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> WasmResult<()> {
        self.ensure_instance().await?;
        let token = self
            .store
            .data_mut()
            .alloc_invocation(ctx.clone(), host_abi.clone());
        let inv = invocation_ctx_to_wit(&ctx, token);
        let bindings = &self.bindings;
        let region = self
            .store
            .run_concurrent(async move |accessor| {
                bindings
                    .actr_workload_workload()
                    .call_on_start(accessor, inv)
                    .await
            })
            .await;
        self.finish_lifecycle("on_start", token, region)
    }

    pub(crate) async fn call_on_ready(
        &mut self,
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> WasmResult<()> {
        self.ensure_instance().await?;
        let token = self
            .store
            .data_mut()
            .alloc_invocation(ctx.clone(), host_abi.clone());
        let inv = invocation_ctx_to_wit(&ctx, token);
        let bindings = &self.bindings;
        let region = self
            .store
            .run_concurrent(async move |accessor| {
                bindings
                    .actr_workload_workload()
                    .call_on_ready(accessor, inv)
                    .await
            })
            .await;
        self.finish_lifecycle("on_ready", token, region)
    }

    pub(crate) async fn call_on_stop(
        &mut self,
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> WasmResult<()> {
        self.ensure_instance().await?;
        let token = self
            .store
            .data_mut()
            .alloc_invocation(ctx.clone(), host_abi.clone());
        let inv = invocation_ctx_to_wit(&ctx, token);
        let bindings = &self.bindings;
        let region = self
            .store
            .run_concurrent(async move |accessor| {
                bindings
                    .actr_workload_workload()
                    .call_on_stop(accessor, inv)
                    .await
            })
            .await;
        self.finish_lifecycle("on_stop", token, region)
    }

    /// Retire the token and classify a fallible-hook region outcome: outer
    /// `Err`/inner trap → poison+rebuild; inner business `Err` →
    /// `ExecutionFailed` (does NOT poison).
    fn finish_lifecycle(
        &mut self,
        label: &str,
        token: u64,
        region: wasmtime::Result<wasmtime::Result<Result<(), WitActrError>>>,
    ) -> WasmResult<()> {
        if !self.poisoned {
            self.store.data_mut().remove_invocation(token);
        }
        match region {
            Err(trap) => Err(self.trap_poison(label, trap)),
            Ok(call_result) => match call_result {
                Ok(inner) => inner.map_err(|e| {
                    WasmError::ExecutionFailed(format!(
                        "{label} error: {:?}",
                        wit_actr_error_to_proto(e)
                    ))
                }),
                Err(trap) => Err(self.trap_poison(label, trap)),
            },
        }
    }

    /// Drive one DataChunk fast-path chunk.
    pub(crate) async fn handle_data_chunk(
        &mut self,
        chunk: DataChunk,
        sender: ActrId,
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> WasmResult<()> {
        self.ensure_instance().await?;
        let wit_chunk = proto_data_chunk_to_wit(chunk);
        let wit_sender = proto_actr_id_to_wit(&sender);
        let token = self
            .store
            .data_mut()
            .alloc_invocation(ctx.clone(), host_abi.clone());
        let inv = invocation_ctx_to_wit(&ctx, token);

        let bindings = &self.bindings;
        let region = self
            .store
            .run_concurrent(async move |accessor| {
                bindings
                    .actr_workload_workload()
                    .call_on_data_chunk(accessor, wit_chunk, wit_sender, inv)
                    .await
            })
            .await;

        if !self.poisoned {
            self.store.data_mut().remove_invocation(token);
        }

        match region {
            Err(trap) => Err(self.trap_poison("on_data_chunk", trap)),
            Ok(call) => match call {
                Ok(inner) => inner.map_err(|e| {
                    WasmError::ExecutionFailed(format!(
                        "on_data_chunk error: {:?}",
                        wit_actr_error_to_proto(e)
                    ))
                }),
                Err(trap) => Err(self.trap_poison("on_data_chunk", trap)),
            },
        }
    }

    /// Drive one infallible observation hook (the twelve `ctx-token`-only
    /// exports). The token is registered so the hook's own host imports
    /// (e.g. `ctx.call_raw`) resolve their `HostAbiFn`.
    pub(crate) async fn call_hook_event(
        &mut self,
        event: PackageHookEvent,
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> WasmResult<()> {
        self.ensure_instance().await?;
        let label = event.request_id();
        let token = self
            .store
            .data_mut()
            .alloc_invocation(ctx, host_abi.clone());

        let bindings = &self.bindings;
        let region = self
            .store
            .run_concurrent(async move |accessor| {
                run_hook_region(accessor, bindings, event, token).await
            })
            .await;

        if !self.poisoned {
            self.store.data_mut().remove_invocation(token);
        }

        match region {
            Err(trap) => Err(self.trap_poison(label, trap)),
            Ok(inner) => inner.map_err(|trap| self.trap_poison(label, trap)),
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // M5 — open concurrency: the resident `run_concurrent` region runner
    // ─────────────────────────────────────────────────────────────────────

    /// Drive this instance as a **resident** `run_concurrent` region for the
    /// whole life of the runner task, so several distinct-conflict-key
    /// dispatches are genuinely in flight on the ONE wasm instance at once
    /// and interleave at their host-import `.await` points (the M0 spike's
    /// Mechanism-1 substrate).
    ///
    /// Ownership contract (kept transparent to [`crate::executor`]): `self` is
    /// consumed and moved into the runner task; `cmd_rx` is the *same* command
    /// channel the serial `run_loop` would own, carrying frozen [`ActorCmd`]s.
    /// A `Dispatch` is pushed into a `FuturesUnordered` inside the live region;
    /// every other command is a **barrier** (drain in-flight, run alone,
    /// resume), preserving lifecycle ordering and the single-runner invariant.
    ///
    /// # Concurrency width
    ///
    /// The runner sets **no** width cap and never inspects conflict keys: the
    /// upstream scheduler's budget `C` is the single source of truth for how
    /// many dispatches are ever in flight (it only re-arms a key after that
    /// key's reply resolves), so `FuturesUnordered` is naturally `≤ C` and
    /// same-key FIFO is enforced one layer up.
    ///
    /// # Fault isolation
    ///
    /// The reply ledger lives **outside** the region (`Arc<Mutex<..>>`, locked
    /// only for momentary insert/remove, never across an `.await`). A guest
    /// trap collapses the entire region to an outer `Err` and drops every
    /// in-region future (the spike's Q3 whole-region teardown), so the ledger
    /// is the only place the in-flight siblings' reply senders survive: on a
    /// trap the supervisor drains it, fails every pending reply with a
    /// retryable error, poisons + clears the invocation table, rebuilds a
    /// fresh store, and re-enters a new region with the *same* `cmd_rx` (the
    /// command queue is plain Rust data a trap cannot destroy).
    pub(crate) async fn run_interleaved(
        mut self,
        mut cmd_rx: mpsc::Receiver<ActorCmd>,
        dispatch_timeout: Option<Duration>,
    ) {
        // Region-external reply ledger (hard constraint): survives a region
        // trap so no caller is left hanging when the whole region collapses.
        let ledger: Arc<Mutex<HashMap<u64, PendingReply>>> = Arc::new(Mutex::new(HashMap::new()));

        loop {
            // Rebuild a poisoned store before (re)entering the region.
            if let Err(e) = self.ensure_instance().await {
                tracing::error!(
                    error = %e,
                    "v2 interleaved runner: store rebuild failed; terminating runner \
                     and failing all pending replies"
                );
                drain_and_fail(
                    &ledger,
                    ActrError::Unavailable("actor instance unrecoverable after trap".to_string()),
                );
                return;
            }

            let bindings = &self.bindings;
            let ledger_region = Arc::clone(&ledger);
            let cmd_rx_ref = &mut cmd_rx;

            let region: wasmtime::Result<RegionExit> = self
                .store
                .run_concurrent(async move |accessor| {
                    resident_region(
                        accessor,
                        bindings,
                        cmd_rx_ref,
                        &ledger_region,
                        dispatch_timeout,
                    )
                    .await
                })
                .await;

            match region {
                // Clean exit: `cmd_rx` closed or an explicit `Shutdown`. All
                // work drained and replied before we got here.
                Ok(RegionExit::Closed) | Ok(RegionExit::Shutdown) => return,
                // A guest trap tore the whole region down. Fail every in-flight
                // sibling, poison, and loop to rebuild + re-enter.
                Err(trap) => self.fail_all_and_poison(&ledger, trap),
            }
        }
    }

    /// Trap recovery for the resident region: fail every still-pending reply in
    /// the region-external ledger (a trap is a whole-instance fault — siblings
    /// are collateral, not adjudicated individually), then poison + clear the
    /// invocation table so the next loop iteration rebuilds a fresh store.
    fn fail_all_and_poison(
        &mut self,
        ledger: &Arc<Mutex<HashMap<u64, PendingReply>>>,
        trap: wasmtime::Error,
    ) {
        let failed = drain_and_fail(
            ledger,
            ActrError::Unavailable("actor instance trapped; message may be retried".to_string()),
        );
        self.poisoned = true;
        self.store.data_mut().clear_invocations();
        tracing::error!(
            error = %trap,
            failed_siblings = failed,
            rebuild_attempt = self.rebuilds + 1,
            "wasm guest trapped (v2 interleaved region); whole region collapsed, \
             all in-flight siblings failed with a retryable error; store poisoned, \
             rebuilding a fresh instance"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Resident-region internals (M5)
// ─────────────────────────────────────────────────────────────────────────────

/// One live invocation's reply sender, parked in the region-external ledger.
/// `Dispatch` replies carry `Bytes`; every barrier reply is unit.
enum PendingReply {
    Dispatch(oneshot::Sender<ActorResult<Bytes>>),
    Unit(oneshot::Sender<ActorResult<()>>),
}

impl PendingReply {
    fn fail(self, err: ActrError) {
        match self {
            PendingReply::Dispatch(tx) => {
                let _ = tx.send(Err(err));
            }
            PendingReply::Unit(tx) => {
                let _ = tx.send(Err(err));
            }
        }
    }
}

/// Non-trap ways a resident region ends. A trap is *not* here — it surfaces as
/// the outer `run_concurrent` `Err`.
enum RegionExit {
    /// `cmd_rx` closed (all handles dropped) and in-flight drained.
    Closed,
    /// Explicit `ActorCmd::Shutdown` barrier.
    Shutdown,
}

/// What the select loop should do after a barrier finishes.
enum BarrierNext {
    Continue,
    Shutdown,
}

/// The outcome of one in-flight dispatch future: either the guest call
/// completed (with its trappable double-`Result`) or the per-dispatch deadline
/// fired and the `call_concurrent` future was dropped mid-flight (CLEAN cancel,
/// spike T8/R1).
enum DispatchOutcome {
    Completed(wasmtime::Result<Result<Vec<u8>, WitActrError>>),
    TimedOut,
}

/// Drain the region-external ledger, failing every pending reply. Returns how
/// many were failed. Used on trap recovery and on unrecoverable teardown.
fn drain_and_fail(ledger: &Arc<Mutex<HashMap<u64, PendingReply>>>, err: ActrError) -> usize {
    let drained: Vec<PendingReply> = {
        let mut guard = ledger.lock().expect("reply ledger mutex poisoned");
        guard.drain().map(|(_, v)| v).collect()
    };
    let n = drained.len();
    for pending in drained {
        pending.fail(err.clone());
    }
    n
}

/// Map a completed dispatch outcome to the caller-facing [`ActorResult`],
/// matching the serial `handle` path's error shaping so gate-off and gate-on
/// are behaviourally identical for the same guest.
fn classify_dispatch(outcome: DispatchOutcome) -> ActorResult<Bytes> {
    match outcome {
        DispatchOutcome::TimedOut => Err(ActrError::TimedOut),
        DispatchOutcome::Completed(Ok(Ok(bytes))) => Ok(Bytes::from(bytes)),
        DispatchOutcome::Completed(Ok(Err(wit_err))) => Err(ActrError::Internal(format!(
            "workload dispatch failed: guest dispatch returned error: {:?}",
            wit_actr_error_to_proto(wit_err)
        ))),
        // A per-future inner `Err` is a non-poisoning host-import bridge fault;
        // a genuine guest trap instead collapses the whole region (handled by
        // the outer `Err` arm), so it never reaches here.
        DispatchOutcome::Completed(Err(trap)) => Err(ActrError::Internal(format!(
            "workload dispatch failed: {trap}"
        ))),
    }
}

/// Map a completed barrier (lifecycle / data-chunk) outcome to a unit reply,
/// mirroring the serial path's `workload {label} failed: ...` shaping.
fn classify_unit(label: &str, res: wasmtime::Result<Result<(), WitActrError>>) -> ActorResult<()> {
    match res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(wit_err)) => Err(ActrError::Internal(format!(
            "workload {label} failed: {:?}",
            wit_actr_error_to_proto(wit_err)
        ))),
        Err(trap) => Err(ActrError::Internal(format!(
            "workload {label} failed: {trap}"
        ))),
    }
}

/// The resident `select!` loop that runs *inside* the live `run_concurrent`
/// region. Accepts new commands off `cmd_rx`, drives dispatches concurrently in
/// a `FuturesUnordered`, and runs every non-dispatch command as a barrier.
async fn resident_region(
    accessor: &Accessor<HostState>,
    bindings: &ActrWorkloadGuestV2,
    cmd_rx: &mut mpsc::Receiver<ActorCmd>,
    ledger: &Arc<Mutex<HashMap<u64, PendingReply>>>,
    dispatch_timeout: Option<Duration>,
) -> RegionExit {
    // One long-lived workload proxy for the whole region: each in-flight
    // `call_dispatch` future borrows it, so it must outlive the
    // `FuturesUnordered` (hoisted, exactly as the M0 spike does).
    let wl = bindings.actr_workload_workload();
    let mut inflight: FuturesUnordered<BoxFuture<'_, (u64, DispatchOutcome)>> =
        FuturesUnordered::new();
    let mut open = true;
    let mut pending_barrier: Option<ActorCmd> = None;

    loop {
        // A queued barrier runs alone, only once every in-flight dispatch has
        // drained — this is the single-runner + lifecycle-ordering guarantee.
        if pending_barrier.is_some() && inflight.is_empty() {
            let barrier = pending_barrier.take().expect("checked is_some");
            match run_barrier(accessor, bindings, ledger, barrier).await {
                BarrierNext::Continue => continue,
                BarrierNext::Shutdown => return RegionExit::Shutdown,
            }
        }
        if !open && inflight.is_empty() && pending_barrier.is_none() {
            return RegionExit::Closed;
        }

        tokio::select! {
            biased;
            // Stop pulling new commands while a barrier is draining.
            maybe_cmd = cmd_rx.recv(), if open && pending_barrier.is_none() => {
                match maybe_cmd {
                    None => open = false,
                    Some(ActorCmd::Dispatch { envelope, ctx, invocation, host_abi, span, reply }) => {
                        // `ctx` (RuntimeContext) drives only the Linked path.
                        let _ = ctx;
                        let wit_env = rpc_envelope_to_wit(&envelope);
                        // Token allocation moves inside the region: the store is
                        // owned by the region, so we go through the accessor (the
                        // host-import path already does the same).
                        let token = accessor
                            .with(|mut a| a.get().alloc_invocation(invocation.clone(), host_abi));
                        let inv = invocation_ctx_to_wit(&invocation, token);
                        ledger
                            .lock()
                            .expect("reply ledger mutex poisoned")
                            .insert(token, PendingReply::Dispatch(reply));
                        let call = wl.call_dispatch(accessor, wit_env, inv);
                        let fut = async move {
                            let out = match dispatch_timeout {
                                // Layer 2 (real cancel): on expiry `timeout`
                                // drops `call`, dropping the in-flight
                                // `call_concurrent` future mid guest-await — a
                                // CLEAN cancel (spike T8/R1), so the store is not
                                // poisoned and siblings are untouched.
                                Some(d) => match tokio::time::timeout(d, call).await {
                                    Ok(r) => DispatchOutcome::Completed(r),
                                    Err(_elapsed) => DispatchOutcome::TimedOut,
                                },
                                None => DispatchOutcome::Completed(call.await),
                            };
                            (token, out)
                        };
                        inflight.push(fut.instrument(span).boxed());
                    }
                    Some(barrier) => pending_barrier = Some(barrier),
                }
            }
            Some((token, outcome)) = inflight.next(), if !inflight.is_empty() => {
                accessor.with(|mut a| a.get().remove_invocation(token));
                let pending = ledger
                    .lock()
                    .expect("reply ledger mutex poisoned")
                    .remove(&token);
                if let Some(PendingReply::Dispatch(tx)) = pending {
                    let _ = tx.send(classify_dispatch(outcome));
                }
            }
        }
    }
}

/// Run one barrier command alone inside the region (in-flight already drained).
/// Registers its reply in the ledger *before* the guest call so a trap during
/// the barrier fails it via the outer supervisor.
async fn run_barrier(
    accessor: &Accessor<HostState>,
    bindings: &ActrWorkloadGuestV2,
    ledger: &Arc<Mutex<HashMap<u64, PendingReply>>>,
    cmd: ActorCmd,
) -> BarrierNext {
    match cmd {
        ActorCmd::Lifecycle {
            phase,
            ctx,
            invocation,
            host_abi,
            span,
            reply,
        } => {
            let _ = ctx;
            let token =
                accessor.with(|mut a| a.get().alloc_invocation(invocation.clone(), host_abi));
            let inv = invocation_ctx_to_wit(&invocation, token);
            ledger
                .lock()
                .expect("reply ledger mutex poisoned")
                .insert(token, PendingReply::Unit(reply));
            let res = run_lifecycle_region(accessor, bindings, phase, inv)
                .instrument(span)
                .await;
            accessor.with(|mut a| a.get().remove_invocation(token));
            if let Some(PendingReply::Unit(tx)) = ledger
                .lock()
                .expect("reply ledger mutex poisoned")
                .remove(&token)
            {
                let _ = tx.send(classify_unit(phase.panic_label(), res));
            }
            BarrierNext::Continue
        }
        ActorCmd::DataChunk {
            chunk,
            sender,
            invocation,
            host_abi,
            span,
            reply,
        } => {
            let token =
                accessor.with(|mut a| a.get().alloc_invocation(invocation.clone(), host_abi));
            let wit_chunk = proto_data_chunk_to_wit(chunk);
            let wit_sender = proto_actr_id_to_wit(&sender);
            let inv = invocation_ctx_to_wit(&invocation, token);
            ledger
                .lock()
                .expect("reply ledger mutex poisoned")
                .insert(token, PendingReply::Unit(reply));
            let res = run_data_chunk_region(accessor, bindings, wit_chunk, wit_sender, inv)
                .instrument(span)
                .await;
            accessor.with(|mut a| a.get().remove_invocation(token));
            if let Some(PendingReply::Unit(tx)) = ledger
                .lock()
                .expect("reply ledger mutex poisoned")
                .remove(&token)
            {
                let _ = tx.send(classify_unit("on_data_chunk", res));
            }
            BarrierNext::Continue
        }
        ActorCmd::Hook {
            event,
            invocation,
            host_abi,
            span,
            reply,
        } => {
            let label = event.request_id();
            let token = accessor.with(|mut a| a.get().alloc_invocation(invocation, host_abi));
            ledger
                .lock()
                .expect("reply ledger mutex poisoned")
                .insert(token, PendingReply::Unit(reply));
            let res = run_hook_region(accessor, bindings, event, token)
                .instrument(span)
                .await;
            accessor.with(|mut a| a.get().remove_invocation(token));
            if let Some(PendingReply::Unit(tx)) = ledger
                .lock()
                .expect("reply ledger mutex poisoned")
                .remove(&token)
            {
                let _ = tx.send(res.map_err(|trap| {
                    ActrError::Internal(format!("workload {label} failed: {trap}"))
                }));
            }
            BarrierNext::Continue
        }
        ActorCmd::Shutdown { done } => {
            if let Some(done) = done {
                let _ = done.send(());
            }
            BarrierNext::Shutdown
        }
        // `Dispatch` is never routed here — it is handled in the select loop.
        ActorCmd::Dispatch { reply, .. } => {
            let _ = reply.send(Err(ActrError::Internal(
                "internal: dispatch reached the barrier path".to_string(),
            )));
            BarrierNext::Continue
        }
    }
}

/// Region-internal lifecycle-hook call (shared by the per-region serial methods
/// and the interleaved barrier path).
async fn run_lifecycle_region(
    accessor: &Accessor<HostState>,
    bindings: &ActrWorkloadGuestV2,
    phase: LifecyclePhase,
    inv: WitInvocationCtx,
) -> wasmtime::Result<Result<(), WitActrError>> {
    let wl = bindings.actr_workload_workload();
    match phase {
        LifecyclePhase::OnStart => wl.call_on_start(accessor, inv).await,
        LifecyclePhase::OnReady => wl.call_on_ready(accessor, inv).await,
        LifecyclePhase::OnStop => wl.call_on_stop(accessor, inv).await,
    }
}

/// Region-internal data-chunk call.
async fn run_data_chunk_region(
    accessor: &Accessor<HostState>,
    bindings: &ActrWorkloadGuestV2,
    wit_chunk: WitDataChunk,
    wit_sender: WitActrId,
    inv: WitInvocationCtx,
) -> wasmtime::Result<Result<(), WitActrError>> {
    bindings
        .actr_workload_workload()
        .call_on_data_chunk(accessor, wit_chunk, wit_sender, inv)
        .await
}

/// Region-internal observation-hook call. Extracted from `call_hook_event` so
/// both the per-region serial path (M4) and the interleaved barrier path (M5)
/// share exactly one copy of the twelve-arm dispatch.
async fn run_hook_region(
    accessor: &Accessor<HostState>,
    bindings: &ActrWorkloadGuestV2,
    event: PackageHookEvent,
    token: u64,
) -> wasmtime::Result<()> {
    let wl = bindings.actr_workload_workload();
    match event {
        PackageHookEvent::SignalingConnecting => {
            wl.call_on_signaling_connecting(accessor, token).await
        }
        PackageHookEvent::SignalingConnected => {
            wl.call_on_signaling_connected(accessor, token).await
        }
        PackageHookEvent::SignalingDisconnected => {
            wl.call_on_signaling_disconnected(accessor, token).await
        }
        PackageHookEvent::WebSocketConnecting(event) => {
            wl.call_on_websocket_connecting(accessor, proto_peer_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::WebSocketConnected(event) => {
            wl.call_on_websocket_connected(accessor, proto_peer_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::WebSocketDisconnected(event) => {
            wl.call_on_websocket_disconnected(accessor, proto_peer_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::WebRtcConnecting(event) => {
            wl.call_on_webrtc_connecting(accessor, proto_peer_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::WebRtcConnected(event) => {
            wl.call_on_webrtc_connected(accessor, proto_peer_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::WebRtcDisconnected(event) => {
            wl.call_on_webrtc_disconnected(accessor, proto_peer_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::CredentialRenewed(event) => {
            wl.call_on_credential_renewed(accessor, proto_credential_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::CredentialExpiring(event) => {
            wl.call_on_credential_expiring(accessor, proto_credential_event_to_wit(event), token)
                .await
        }
        PackageHookEvent::MailboxBackpressure(event) => {
            wl.call_on_mailbox_backpressure(accessor, proto_backpressure_event_to_wit(event), token)
                .await
        }
    }
}
