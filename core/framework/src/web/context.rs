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
//! `call_raw` / `call` / `tell` / `discover_route_candidate` thread
//! `self.request_id()` into the `actr_web_abi::guest::*_with_request_id`
//! wrappers. The sw-host `DISPATCH_CTXS` HashMap (γ-unified §3.6) keys
//! every per-dispatch `RuntimeContext` by that string, so multiple
//! concurrent dispatches never cross wires on the shared JS thread.
//!
//! Only `Dest::Actor(_)` is routable from the browser right now — the
//! `Shell` / `Local` variants have no meaning once the guest runs in a
//! wasm module and the host lives in the service worker. Callers get
//! `ActrError::NotImplemented` for those two shapes; if a future phase
//! adds same-realm shortcuts we can revisit.
//!
//! DataStream / MediaTrack fast paths are not part of Phase 6 γ and
//! remain permanently `NotImplemented` on the web target.

use std::rc::Rc;

use actr_protocol::{
    ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType, Realm, RpcRequest,
};
use async_trait::async_trait;
use futures_util::future::BoxFuture;
use prost::Message as ProstMessage;

use crate::{Context, Dest, LogLevel, MediaSample};

// Pull in the WIT-lowered mirror types from `actr-web-abi` under aliases
// so the guest-import call sites don't have to disambiguate per line.
use actr_web_abi::types as wit;

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

// ── WIT ⇄ protocol value conversions ───────────────────────────────────
//
// The WIT-lowered mirror types in `actr_web_abi::types` are structurally
// identical to the protocol ones but live in a different crate (and
// therefore a different type). Every call that crosses the host-import
// boundary rebuilds the value; these helpers keep the boilerplate out of
// the trait method bodies.

fn actr_type_to_wit(t: &ActrType) -> wit::ActrType {
    wit::ActrType {
        manufacturer: t.manufacturer.clone(),
        name: t.name.clone(),
        version: t.version.clone(),
    }
}

fn actr_type_from_wit(t: &wit::ActrType) -> ActrType {
    ActrType {
        manufacturer: t.manufacturer.clone(),
        name: t.name.clone(),
        version: t.version.clone(),
    }
}

fn actr_id_to_wit(id: &ActrId) -> wit::ActrId {
    wit::ActrId {
        realm: wit::Realm {
            realm_id: id.realm.realm_id,
        },
        serial_number: id.serial_number,
        actr_type: actr_type_to_wit(&id.r#type),
    }
}

fn actr_id_from_wit(id: &wit::ActrId) -> ActrId {
    ActrId {
        realm: Realm {
            realm_id: id.realm.realm_id,
        },
        serial_number: id.serial_number,
        r#type: actr_type_from_wit(&id.actr_type),
    }
}

fn wit_error_to_proto(e: wit::ActrError) -> ActrError {
    match e {
        wit::ActrError::Unavailable(m) => ActrError::Unavailable(m),
        wit::ActrError::TimedOut => ActrError::TimedOut,
        wit::ActrError::NotFound(m) => ActrError::NotFound(m),
        wit::ActrError::PermissionDenied(m) => ActrError::PermissionDenied(m),
        wit::ActrError::InvalidArgument(m) => ActrError::InvalidArgument(m),
        wit::ActrError::UnknownRoute(m) => ActrError::UnknownRoute(m),
        wit::ActrError::DependencyNotFound(p) => {
            // DependencyNotFound carries a {service_name, message} pair on
            // both sides; flatten into the protocol shape.
            ActrError::DependencyNotFound {
                service_name: p.service_name,
                message: p.message,
            }
        }
        wit::ActrError::DecodeFailure(m) => ActrError::DecodeFailure(m),
        wit::ActrError::NotImplemented(m) => ActrError::NotImplemented(m),
        wit::ActrError::Internal(m) => ActrError::Internal(m),
    }
}

/// Collapse the double-Result returned by every
/// `actr_web_abi::guest::*_with_request_id` wrapper into a single
/// [`ActorResult`]. The outer `Result<_, JsValue>` reflects a JS-side
/// trap (serde marshaling, undefined host fn), the inner
/// `Result<T, wit::ActrError>` is the WIT-declared return variant. Both
/// are errors from the caller's perspective.
fn flatten_js<T>(outcome: Result<Result<T, wit::ActrError>, wasm_bindgen::JsValue>) -> ActorResult<T> {
    match outcome {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(wit_error_to_proto(e)),
        Err(js) => Err(ActrError::Internal(format!(
            "WebContext: host import trap: {}",
            js.as_string().unwrap_or_else(|| format!("{js:?}"))
        ))),
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

    async fn call<R: RpcRequest>(&self, target: &Dest, request: R) -> ActorResult<R::Response> {
        // Typed convenience: encode request via prost, delegate to call_raw,
        // decode response back to the typed Response. Mirrors the wasip2
        // `WasmContext::call` path so handler code is target-agnostic.
        let actor = match target {
            Dest::Actor(id) => id,
            Dest::Shell | Dest::Local => {
                return Err(Self::not_implemented("call → Shell/Local dest"));
            }
        };
        let payload = request.encode_to_vec();
        let bytes = self
            .call_raw(actor, R::route_key(), bytes::Bytes::from(payload))
            .await?;
        R::Response::decode(bytes.as_ref()).map_err(|e| {
            ActrError::DecodeFailure(format!(
                "WebContext::call: response decode failed: {e}"
            ))
        })
    }

    async fn tell<R: RpcRequest>(&self, target: &Dest, message: R) -> ActorResult<()> {
        // Fire-and-forget: route through `tell_with_request_id` so the host
        // can short-circuit without a response round-trip. Dest conversion
        // matches the guest-import contract exactly (§3.4).
        let payload = message.encode_to_vec();
        let wit_dest = match target {
            Dest::Shell => wit::Dest::Shell,
            Dest::Local => wit::Dest::Local,
            Dest::Actor(id) => wit::Dest::Actor(actr_id_to_wit(id)),
        };
        let outcome = actr_web_abi::guest::tell_with_request_id(
            self.request_id(),
            wit_dest,
            R::route_key().to_string(),
            payload,
        )
        .await;
        flatten_js(outcome)
    }

    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: bytes::Bytes,
    ) -> ActorResult<bytes::Bytes> {
        // The raw entry point is the one the typed `call` delegates to and
        // the only shape that actually traverses the sw-host HashMap
        // keyed by `request_id` (γ-unified §3.6).
        let outcome = actr_web_abi::guest::call_raw_with_request_id(
            self.request_id(),
            actr_id_to_wit(target),
            route_key.to_string(),
            payload.to_vec(),
        )
        .await;
        flatten_js(outcome).map(bytes::Bytes::from)
    }

    async fn discover_route_candidate(&self, target_type: &ActrType) -> ActorResult<ActrId> {
        // Signalling lookup. Even though discovery doesn't depend on the
        // per-dispatch runtime context today, we still thread the
        // `request_id` so the host import signatures stay uniform and the
        // sw-host can attribute the lookup to the correct dispatch in
        // logs / metrics.
        let outcome = actr_web_abi::guest::discover_with_request_id(
            self.request_id(),
            actr_type_to_wit(target_type),
        )
        .await;
        flatten_js(outcome).map(|wit_id| actr_id_from_wit(&wit_id))
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
