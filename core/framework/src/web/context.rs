//! `WebContext` — wasm-bindgen path `Context` implementation.
//!
//! Per Option U γ-unified §3.3 this is the browser-side counterpart of the
//! native `RuntimeContext` and the `wasip2` `WasmContext`. It is created
//! once per inbound dispatch (by the entry-point glue generated from
//! `register_workload`), binds `(self_id, caller_id, request_id)` at
//! construction time, and is cloned freely into handler futures.
//!
//! # Concurrency model
//!
//! The browser is single-threaded (JS event loop). `WebContext` therefore
//! wraps its state in `Rc` rather than `Arc` and intentionally does **not**
//! implement `Send` / `Sync`. The framework `Context` trait is `?Send` on
//! `wasm32`, so handler futures compose without fighting the auto traits.
//!
//! # Where the RPC methods actually route
//!
//! `call_raw` / `call` / `tell` / `discover_route_candidate` are intentional
//! `todo!()` placeholders on this commit: the wasm-bindgen host surface
//! (`actr_web_abi::guest::call_raw_with_request_id` et al.) is being
//! regenerated in parallel by agent P6-C. Once that lands, agent P6-I
//! wires each method to the corresponding host import.
//!
//! DataStream / MediaTrack fast paths are not part of Phase 6 γ and
//! remain permanently `NotImplemented` on the web target.

use std::rc::Rc;

use actr_protocol::{
    ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType, RpcRequest,
};
use async_trait::async_trait;
use futures_util::future::BoxFuture;

use crate::{Context, Dest, LogLevel, MediaSample};

/// Inner state shared by clones of a [`WebContext`].
///
/// Kept behind an [`Rc`] so handler closures cloning the context do not
/// reallocate the identity fields.
struct WebContextInner {
    self_id: ActrId,
    caller_id: Option<ActrId>,
    /// Per-dispatch request id. Supplied by the host bridge when the
    /// workload is invoked; every outgoing `call_raw` from this context
    /// carries the same id so the sw-host `DISPATCH_CTXS` HashMap can
    /// find the right runtime context (see γ-unified §3.6).
    request_id: String,
}

/// Web-target `Context` implementation.
///
/// Cloning is `Rc::clone` — cheap and single-threaded.
#[derive(Clone)]
pub struct WebContext {
    inner: Rc<WebContextInner>,
}

impl WebContext {
    /// Build a new context bound to a single inbound dispatch.
    ///
    /// Constructed by the wasm-bindgen entry-point glue (`register_workload`
    /// in `actr-web-abi`) for every call the host dispatches. Users never
    /// call this directly.
    pub fn new(self_id: ActrId, caller_id: Option<ActrId>, request_id: String) -> Self {
        Self {
            inner: Rc::new(WebContextInner {
                self_id,
                caller_id,
                request_id,
            }),
        }
    }

    fn not_implemented(feature: &'static str) -> ActrError {
        ActrError::NotImplemented(format!("WebContext::{feature}"))
    }
}

#[async_trait(?Send)]
impl Context for WebContext {
    // ── Identity ────────────────────────────────────────────────────────

    fn self_id(&self) -> &ActrId {
        &self.inner.self_id
    }

    fn caller_id(&self) -> Option<&ActrId> {
        self.inner.caller_id.as_ref()
    }

    fn request_id(&self) -> &str {
        &self.inner.request_id
    }

    // ── Communication ───────────────────────────────────────────────────
    //
    // The four methods below will route through actr-web-abi host imports
    // once agent P6-C regenerates guest.rs with the request_id-carrying
    // signatures (γ-unified §3.4). Agent P6-I wires them up during the
    // integration phase. Leaving them as `todo!()` here lets dependents
    // type-check against the contract while the integration lands.

    async fn call<R: RpcRequest>(&self, _target: &Dest, _request: R) -> ActorResult<R::Response> {
        // P6-I: route through `actr_web_abi::guest::call_raw_with_request_id`
        // then decode the response via `R::Response::decode`.
        todo!("WebContext::call — wired up in P6-I integration")
    }

    async fn tell<R: RpcRequest>(&self, _target: &Dest, _message: R) -> ActorResult<()> {
        // P6-I: route through `actr_web_abi::guest::tell_with_request_id`.
        todo!("WebContext::tell — wired up in P6-I integration")
    }

    async fn call_raw(
        &self,
        _target: &ActrId,
        _route_key: &str,
        _payload: bytes::Bytes,
    ) -> ActorResult<bytes::Bytes> {
        // P6-I: forward `self.request_id()` + target + route_key + payload
        // to `actr_web_abi::guest::call_raw_with_request_id` (§3.4).
        todo!("WebContext::call_raw — wired up in P6-I integration")
    }

    async fn discover_route_candidate(&self, _target_type: &ActrType) -> ActorResult<ActrId> {
        // P6-I: forward to `actr_web_abi::guest::discover_with_request_id`.
        todo!("WebContext::discover_route_candidate — wired up in P6-I integration")
    }

    // ── DataStream fast path (not supported on web) ─────────────────────

    async fn register_stream<F>(&self, _stream_id: String, _callback: F) -> ActorResult<()>
    where
        F: Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        Err(Self::not_implemented("register_stream"))
    }

    async fn unregister_stream(&self, _stream_id: &str) -> ActorResult<()> {
        Err(Self::not_implemented("unregister_stream"))
    }

    async fn send_data_stream(
        &self,
        _target: &Dest,
        _chunk: DataStream,
        _payload_type: PayloadType,
    ) -> ActorResult<()> {
        Err(Self::not_implemented("send_data_stream"))
    }

    // ── MediaTrack fast path (WebRTC native, not available to web guests) ──

    async fn register_media_track<F>(&self, _track_id: String, _callback: F) -> ActorResult<()>
    where
        F: Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        Err(Self::not_implemented("register_media_track"))
    }

    async fn unregister_media_track(&self, _track_id: &str) -> ActorResult<()> {
        Err(Self::not_implemented("unregister_media_track"))
    }

    async fn send_media_sample(
        &self,
        _target: &Dest,
        _track_id: &str,
        _sample: MediaSample,
    ) -> ActorResult<()> {
        Err(Self::not_implemented("send_media_sample"))
    }

    async fn add_media_track(
        &self,
        _target: &Dest,
        _track_id: &str,
        _codec: &str,
        _media_type: &str,
    ) -> ActorResult<()> {
        Err(Self::not_implemented("add_media_track"))
    }

    async fn remove_media_track(&self, _target: &Dest, _track_id: &str) -> ActorResult<()> {
        Err(Self::not_implemented("remove_media_track"))
    }

    // ── Observation ─────────────────────────────────────────────────────

    fn log(&self, level: LogLevel, msg: &str) {
        // `wasm-bindgen` + `tracing-web` (installed by the runtime glue)
        // route these records to `console.*` automatically; we therefore
        // delegate to the default `tracing` body. If a downstream host
        // wires a different sink (e.g. a host-import log channel) it can
        // override this method in its own wrapper.
        match level {
            LogLevel::Trace => tracing::trace!(target: "actr_framework::workload", "{msg}"),
            LogLevel::Debug => tracing::debug!(target: "actr_framework::workload", "{msg}"),
            LogLevel::Info => tracing::info!(target: "actr_framework::workload", "{msg}"),
            LogLevel::Warn => tracing::warn!(target: "actr_framework::workload", "{msg}"),
            LogLevel::Error => tracing::error!(target: "actr_framework::workload", "{msg}"),
        }
    }
}
