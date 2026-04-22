//! Component Model guest bridge for the Web runtime.
//!
//! # Phase 3 (post-migration) shape
//!
//! The browser runtime consumes user guests as **Component Model** binaries
//! (WIT contract `core/framework/wit/actr-workload.wit`), transpiled by
//! `jco` into an ES module + core wasm bundle. The Service Worker JS loads
//! that bundle via `WebAssembly.instantiate`, obtains the `workload`
//! interface, and wires the runtime-sw-provided host imports into the
//! component's `host` interface.
//!
//! This crate (`actr-runtime-sw`, Rust compiled to wasm32 via wasm-bindgen)
//! provides two roles:
//!
//! 1. **Guest dispatch driver** — [`register_component_workload`] accepts a
//!    JS callback that forwards to the jco-generated `workload.dispatch`
//!    export. The runtime wraps it behind the [`WasmWorkload`] abstraction
//!    used by the inbound packet dispatcher.
//! 2. **Host import implementations** — functions like
//!    [`host_call_raw_async`], [`host_discover_async`], [`host_log_message`]
//!    and the per-dispatch context getters expose the Rust side of the WIT
//!    `actr:workload/host` interface as wasm-bindgen JS functions. The SW
//!    glue JS builds an import object from these and hands it to jco's
//!    `instantiate`.
//!
//! # WIT host interface, as JS function names
//!
//! | WIT                                 | JS function (this module)        |
//! |-------------------------------------|-----------------------------------|
//! | `host.call(target, route, payload)` | [`host_call_async`]              |
//! | `host.tell(target, route, payload)` | [`host_tell_async`]              |
//! | `host.call-raw(id, route, payload)` | [`host_call_raw_async`]          |
//! | `host.discover(type)`               | [`host_discover_async`]          |
//! | `host.log-message(level, message)`  | [`host_log_message`]             |
//! | `host.get-self-id()`                | [`host_get_self_id`]             |
//! | `host.get-caller-id()`              | [`host_get_caller_id`]           |
//! | `host.get-request-id()`             | [`host_get_request_id`]          |
//!
//! # Dispatch flow
//!
//! ```text
//!  inbound RPC ─► WasmWorkload ─► ServiceHandlerFn
//!                                   │
//!                                   ├─ set GUEST_CTX (thread-local)
//!                                   ├─ build envelope JS object
//!                                   ├─ await dispatchFn(envelope)          ─┐
//!                                   │    (jco: workload.dispatch — async)   │
//!                                   │                                       │
//!                                   │      during guest execution, jco      │
//!                                   │      calls back into host_*_async     │
//!                                   │      which reads GUEST_CTX and        │
//!                                   │      routes through RuntimeContext    │
//!                                   │                                       │
//!                                   ├─ clear GUEST_CTX                     ◄┘
//!                                   └─ return reply bytes
//! ```
//!
//! # Legacy path removed
//!
//! The previous handwritten prost `AbiFrame` / `GuestHandleV1` bridge and the
//! cdylib `actr_init` / `actr_handle` entry points are gone from this crate
//! as of the Component Model browser migration. The Rust-side legacy ABI
//! types in `actr_framework::guest::dynclib_abi` remain in use by the native
//! **DynClib** backend and are untouched here.

use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

use actr_framework::Dest;
use actr_protocol::{ActrError, ActrId, ActrType, RpcEnvelope};
use bytes::Bytes;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::context::RuntimeContext;
use crate::web_context::WebContext;
use crate::workload::{ServiceHandlerFn, WasmWorkload};

// ─────────────────────────────────────────────────────────────────────────────
// Per-dispatch context
// ─────────────────────────────────────────────────────────────────────────────

// Thread-local storage for the current `RuntimeContext` during guest
// dispatch. Set before calling the JS dispatch function (which then drives
// the jco-transpiled component), cleared after.
//
// The host-import functions (`host_*_async`) read this to service outbound
// calls triggered by the guest's `.await` on a WIT host import.
thread_local! {
    static GUEST_CTX: RefCell<Option<Rc<RuntimeContext>>> = const { RefCell::new(None) };
}

/// Install a `RuntimeContext` for the duration of a guest dispatch. Panics if
/// one is already installed — the web runtime is single-threaded and
/// dispatches are serialized per actor instance.
fn install_ctx(ctx: Rc<RuntimeContext>) {
    GUEST_CTX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_some() {
            log::error!(
                "[GuestBridge] install_ctx called while another context is active — dispatch overlap?"
            );
        }
        *slot = Some(ctx);
    });
}

/// Clear the per-dispatch context. Safe to call even when no context is set.
fn clear_ctx() {
    GUEST_CTX.with(|cell| cell.replace(None));
}

/// Retrieve the currently installed `RuntimeContext`, returning a JS error
/// string if no dispatch is active. Host-import implementations use this to
/// reject calls originating outside an active dispatch (e.g. lifecycle
/// hooks, which the component model routes through the same exports but
/// under a host-driven context injection).
fn current_ctx() -> Result<Rc<RuntimeContext>, JsValue> {
    GUEST_CTX
        .with(|cell| cell.borrow().clone())
        .ok_or_else(|| JsValue::from_str("no guest context active"))
}

// ─────────────────────────────────────────────────────────────────────────────
// JS <-> Rust type helpers for the WIT surface
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a WIT `actr-id` record (as a JS object) into the protobuf
/// `ActrId`. The JS caller builds this from the jco-generated types shape
/// — `{ realm: { realmId }, serialNumber, type: { manufacturer, name, version } }`
/// — and passes it through to Rust.
fn parse_actr_id(value: &JsValue) -> Result<ActrId, JsValue> {
    let realm = js_sys::Reflect::get(value, &JsValue::from_str("realm"))?;
    let realm_id = js_sys::Reflect::get(&realm, &JsValue::from_str("realmId"))?
        .as_f64()
        .ok_or_else(|| JsValue::from_str("realm.realmId not a number"))? as u32;

    let serial_number = js_sys::Reflect::get(value, &JsValue::from_str("serialNumber"))?;
    let serial_number = if let Some(f) = serial_number.as_f64() {
        f as u64
    } else {
        // jco emits `bigint` for WIT u64; fall back to string parse.
        let s = serial_number
            .as_string()
            .ok_or_else(|| JsValue::from_str("serialNumber not number/string"))?;
        s.parse::<u64>()
            .map_err(|e| JsValue::from_str(&format!("serialNumber parse: {e}")))?
    };

    // `type` collides with the WIT escape (`%type`); jco emits plain `type`
    // on the JS object. Probe both for robustness.
    let ty = js_sys::Reflect::get(value, &JsValue::from_str("type"))?;
    let ty = if ty.is_undefined() {
        js_sys::Reflect::get(value, &JsValue::from_str("%type"))?
    } else {
        ty
    };
    let manufacturer = js_sys::Reflect::get(&ty, &JsValue::from_str("manufacturer"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("type.manufacturer missing"))?;
    let name = js_sys::Reflect::get(&ty, &JsValue::from_str("name"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("type.name missing"))?;
    let version = js_sys::Reflect::get(&ty, &JsValue::from_str("version"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("type.version missing"))?;

    Ok(ActrId {
        realm: actr_protocol::Realm { realm_id },
        serial_number,
        r#type: ActrType {
            manufacturer,
            name,
            version,
        },
    })
}

/// Serialise `ActrId` into the JS-object shape jco expects. Mirror of
/// [`parse_actr_id`].
fn actr_id_to_js(id: &ActrId) -> JsValue {
    let realm_obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &realm_obj,
        &JsValue::from_str("realmId"),
        &JsValue::from_f64(id.realm.realm_id as f64),
    );

    let type_obj = js_sys::Object::new();
    let t = &id.r#type;
    let _ = js_sys::Reflect::set(
        &type_obj,
        &JsValue::from_str("manufacturer"),
        &JsValue::from_str(&t.manufacturer),
    );
    let _ = js_sys::Reflect::set(
        &type_obj,
        &JsValue::from_str("name"),
        &JsValue::from_str(&t.name),
    );
    let _ = js_sys::Reflect::set(
        &type_obj,
        &JsValue::from_str("version"),
        &JsValue::from_str(&t.version),
    );

    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("realm"), &realm_obj);
    // Use `BigInt` for serial-number to match jco's `bigint` u64 representation.
    let serial = js_sys::BigInt::from(id.serial_number);
    let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("serialNumber"), &serial);
    let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("type"), &type_obj);
    obj.into()
}

/// Parse a WIT `dest` variant JS object: `{ tag: 'shell' | 'local' | 'actor', val?: ActrId }`.
fn parse_dest(value: &JsValue) -> Result<Dest, JsValue> {
    let tag = js_sys::Reflect::get(value, &JsValue::from_str("tag"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("dest.tag missing or not a string"))?;
    match tag.as_str() {
        "shell" => Ok(Dest::Shell),
        "local" => Ok(Dest::Local),
        "actor" => {
            let val = js_sys::Reflect::get(value, &JsValue::from_str("val"))?;
            if val.is_undefined() || val.is_null() {
                return Err(JsValue::from_str("dest.actor missing val"));
            }
            Ok(Dest::Actor(parse_actr_id(&val)?))
        }
        other => Err(JsValue::from_str(&format!("unknown dest tag: {other}"))),
    }
}

/// Parse a WIT `actr-type` record JS object.
fn parse_actr_type(value: &JsValue) -> Result<ActrType, JsValue> {
    Ok(ActrType {
        manufacturer: js_sys::Reflect::get(value, &JsValue::from_str("manufacturer"))?
            .as_string()
            .ok_or_else(|| JsValue::from_str("manufacturer missing"))?,
        name: js_sys::Reflect::get(value, &JsValue::from_str("name"))?
            .as_string()
            .ok_or_else(|| JsValue::from_str("name missing"))?,
        version: js_sys::Reflect::get(value, &JsValue::from_str("version"))?
            .as_string()
            .ok_or_else(|| JsValue::from_str("version missing"))?,
    })
}

/// Translate an internal `ActrError` into a JS `Error` suitable for jco's
/// `result<_, actr-error>` error arm. The JS glue in the SW wraps these
/// with the matching variant tags; here we flatten to message text plus a
/// machine-readable `name` attribute so the SW can re-tag deterministically.
fn actr_error_to_js(error: ActrError) -> JsValue {
    let (tag, message) = match &error {
        ActrError::Unavailable(m) => ("unavailable", m.clone()),
        ActrError::TimedOut => ("timed-out", String::new()),
        ActrError::NotFound(m) => ("not-found", m.clone()),
        ActrError::PermissionDenied(m) => ("permission-denied", m.clone()),
        ActrError::InvalidArgument(m) => ("invalid-argument", m.clone()),
        ActrError::UnknownRoute(r) => ("unknown-route", r.clone()),
        ActrError::DependencyNotFound {
            service_name,
            message,
        } => ("dependency-not-found", format!("{service_name}: {message}")),
        ActrError::DecodeFailure(m) => ("decode-failure", m.clone()),
        ActrError::NotImplemented(m) => ("not-implemented", m.clone()),
        ActrError::Internal(m) => ("internal", m.clone()),
    };
    let err = js_sys::Error::new(&format!("{tag}: {message}"));
    let _ = js_sys::Reflect::set(&err, &JsValue::from_str("name"), &JsValue::from_str(tag));
    let _ = js_sys::Reflect::set(
        &err,
        &JsValue::from_str("actrErrorTag"),
        &JsValue::from_str(tag),
    );
    err.into()
}

// ─────────────────────────────────────────────────────────────────────────────
// Public wasm-bindgen surface — host imports
// ─────────────────────────────────────────────────────────────────────────────

/// WIT `host.call-raw(target, route_key, payload) -> result<list<u8>, actr-error>`
///
/// Async; returns a Promise that resolves to a `Uint8Array` on success or
/// rejects with a JS `Error` whose `actrErrorTag` names the WIT variant.
#[wasm_bindgen]
pub async fn host_call_raw_async(
    target: JsValue,
    route_key: String,
    payload: js_sys::Uint8Array,
) -> Result<js_sys::Uint8Array, JsValue> {
    let ctx = current_ctx()?;
    let target = parse_actr_id(&target)?;
    let payload_bytes = payload.to_vec();

    log::debug!(
        "[GuestBridge] host.call-raw target=<{}> route={} len={}",
        target.serial_number,
        route_key,
        payload_bytes.len()
    );

    match ctx
        .call_raw(&target, &route_key, &payload_bytes, 30_000)
        .await
    {
        Ok(response) => Ok(js_sys::Uint8Array::from(&response[..])),
        Err(e) => Err(actr_error_to_js(e)),
    }
}

/// WIT `host.call(target, route_key, payload) -> result<list<u8>, actr-error>`
///
/// The web runtime only supports `dest::actor` for typed calls today (it has
/// no in-browser Shell/Local routing); other variants return
/// `not-implemented`. Keeps the WIT contract uniform between server and
/// browser — the variant arm exists, it just isn't wired.
#[wasm_bindgen]
pub async fn host_call_async(
    target: JsValue,
    route_key: String,
    payload: js_sys::Uint8Array,
) -> Result<js_sys::Uint8Array, JsValue> {
    let ctx = current_ctx()?;
    let dest = parse_dest(&target)?;
    let payload_bytes = payload.to_vec();

    let actor_id = match dest {
        Dest::Actor(id) => id,
        Dest::Shell | Dest::Local => {
            return Err(actr_error_to_js(ActrError::NotImplemented(
                "host.call with Shell/Local dest is unsupported in the web runtime".into(),
            )));
        }
    };

    match ctx
        .call_raw(&actor_id, &route_key, &payload_bytes, 30_000)
        .await
    {
        Ok(response) => Ok(js_sys::Uint8Array::from(&response[..])),
        Err(e) => Err(actr_error_to_js(e)),
    }
}

/// WIT `host.tell(target, route_key, payload) -> result<_, actr-error>`.
///
/// Fire-and-forget semantics. The web runtime maps this to `call_raw` with
/// `timeout_ms=0`; the result is discarded. Only `Dest::Actor` is wired.
#[wasm_bindgen]
pub async fn host_tell_async(
    target: JsValue,
    route_key: String,
    payload: js_sys::Uint8Array,
) -> Result<(), JsValue> {
    let ctx = current_ctx()?;
    let dest = parse_dest(&target)?;
    let payload_bytes = payload.to_vec();

    let actor_id = match dest {
        Dest::Actor(id) => id,
        Dest::Shell | Dest::Local => {
            return Err(actr_error_to_js(ActrError::NotImplemented(
                "host.tell with Shell/Local dest is unsupported in the web runtime".into(),
            )));
        }
    };

    match ctx.call_raw(&actor_id, &route_key, &payload_bytes, 0).await {
        Ok(_) => Ok(()),
        Err(e) => Err(actr_error_to_js(e)),
    }
}

/// WIT `host.discover(target_type) -> result<actr-id, actr-error>`.
#[wasm_bindgen]
pub async fn host_discover_async(target_type: JsValue) -> Result<JsValue, JsValue> {
    let ctx = current_ctx()?;
    let target_type = parse_actr_type(&target_type)?;

    log::debug!(
        "[GuestBridge] host.discover target={}:{}:{}",
        target_type.manufacturer,
        target_type.name,
        target_type.version
    );

    match ctx.discover(&target_type).await {
        Ok(id) => Ok(actr_id_to_js(&id)),
        Err(e) => Err(actr_error_to_js(e)),
    }
}

/// WIT `host.log-message(level, message)`.
///
/// Maps to `log` crate levels. Levels outside the `trace/debug/info/warn/error`
/// set silently fall through to `info`.
#[wasm_bindgen]
pub fn host_log_message(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("[guest] {message}"),
        "warn" => log::warn!("[guest] {message}"),
        "debug" => log::debug!("[guest] {message}"),
        "trace" => log::trace!("[guest] {message}"),
        _ => log::info!("[guest] {message}"),
    }
}

/// WIT `host.get-self-id() -> actr-id`.
#[wasm_bindgen]
pub fn host_get_self_id() -> Result<JsValue, JsValue> {
    let ctx = current_ctx()?;
    Ok(actr_id_to_js(ctx.self_id()))
}

/// WIT `host.get-caller-id() -> option<actr-id>`. Returns `null` when the
/// host did not install a caller for this dispatch (lifecycle hooks).
#[wasm_bindgen]
pub fn host_get_caller_id() -> Result<JsValue, JsValue> {
    let ctx = current_ctx()?;
    Ok(match ctx.caller_id() {
        Some(id) => actr_id_to_js(id),
        None => JsValue::NULL,
    })
}

/// WIT `host.get-request-id() -> string`.
#[wasm_bindgen]
pub fn host_get_request_id() -> Result<String, JsValue> {
    let ctx = current_ctx()?;
    Ok(ctx.request_id().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Component workload registration — guest dispatch surface
// ─────────────────────────────────────────────────────────────────────────────

/// Register a Component Model guest workload.
///
/// `dispatch_fn` is a JS callback that forwards to the jco-transpiled
/// component's `workload.dispatch(envelope)` export. Its signature must match:
///
/// ```text
/// async (envelope: RpcEnvelopeJs) => Uint8Array
/// ```
///
/// where `RpcEnvelopeJs` is the jco-emitted record:
/// `{ requestId: string, routeKey: string, payload: Uint8Array }`.
///
/// The JS side is responsible for:
/// 1. Loading the jco-transpiled ES module (`<name>.js`) and calling its
///    `instantiate(getCoreModule, imports)` with `imports['actr:workload/host@0.1.0']`
///    bound to the `host_*_async` / `host_*` wasm-bindgen exports from this crate.
/// 2. Calling `instantiate(...)` exactly once and holding the returned
///    exports object.
/// 3. Passing `(envelope) => exports['actr:workload/workload@0.1.0'].dispatch(envelope)`
///    here as `dispatch_fn`.
///
/// When this function is invoked the runtime installs the `ServiceHandlerFn`
/// used by [`WasmWorkload`], which the inbound dispatcher drives.
#[wasm_bindgen]
pub fn register_component_workload(dispatch_fn: js_sys::Function) {
    let handler: ServiceHandlerFn = Rc::new(
        move |route_key: &str, body: &[u8], ctx: Rc<RuntimeContext>| {
            let dispatch_fn = dispatch_fn.clone();
            let route_key = route_key.to_string();
            let body = body.to_vec();

            Box::pin(async move {
                log::debug!(
                    "[GuestBridge] dispatch route={} body_len={}",
                    route_key,
                    body.len()
                );

                // Build the jco `rpc-envelope` JS object.
                let envelope_js = js_sys::Object::new();
                let _ = js_sys::Reflect::set(
                    &envelope_js,
                    &JsValue::from_str("requestId"),
                    &JsValue::from_str(ctx.request_id()),
                );
                let _ = js_sys::Reflect::set(
                    &envelope_js,
                    &JsValue::from_str("routeKey"),
                    &JsValue::from_str(&route_key),
                );
                let _ = js_sys::Reflect::set(
                    &envelope_js,
                    &JsValue::from_str("payload"),
                    &js_sys::Uint8Array::from(&body[..]).into(),
                );

                install_ctx(ctx.clone());

                let result = match dispatch_fn.call1(&JsValue::NULL, &envelope_js) {
                    Ok(v) => v,
                    Err(e) => {
                        clear_ctx();
                        return Err(format!("guest dispatch threw: {e:?}"));
                    }
                };

                // jco `workload.dispatch` is async; the return is always a
                // Promise. Await it, then convert to bytes.
                let resolved = if result.is_instance_of::<js_sys::Promise>() {
                    let promise = js_sys::Promise::from(result);
                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                        Ok(v) => v,
                        Err(e) => {
                            clear_ctx();
                            // jco rejects with the jco `Error`-shaped variant;
                            // tag/message was set by `actr_error_to_js` on the
                            // host side, or by the guest-thrown error directly.
                            return Err(format!("guest dispatch rejected: {e:?}"));
                        }
                    }
                } else {
                    // Defensive: treat a sync return as an immediate Uint8Array.
                    result
                };

                clear_ctx();

                if resolved.is_null() || resolved.is_undefined() {
                    return Err("guest dispatch returned null/undefined".to_string());
                }

                let arr = resolved
                    .dyn_into::<js_sys::Uint8Array>()
                    .map_err(|e| format!("guest dispatch did not return Uint8Array: {e:?}"))?;
                Ok(arr.to_vec())
            }) as Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>>>>
        },
    );

    crate::register_workload(WasmWorkload::new(handler));
    log::info!("[SW] Component workload registered via jco bridge");
}

// ─────────────────────────────────────────────────────────────────────────────
// RpcEnvelope helpers (exposed for tests / advanced JS consumers)
// ─────────────────────────────────────────────────────────────────────────────

/// Construct an `RpcEnvelope` in Rust from parts (used internally when the
/// runtime synthesizes envelopes outside the dispatch callback path). Kept
/// private to the crate; not exported to wasm-bindgen.
#[allow(dead_code)]
pub(crate) fn envelope_from_parts(
    request_id: &str,
    route_key: &str,
    payload: Vec<u8>,
    timeout_ms: i64,
) -> RpcEnvelope {
    RpcEnvelope {
        request_id: request_id.to_string(),
        route_key: route_key.to_string(),
        payload: Some(Bytes::from(payload)),
        error: None,
        traceparent: None,
        tracestate: None,
        metadata: vec![],
        timeout_ms,
    }
}
