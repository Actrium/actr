//! Guest WASM bridge for the Web runtime.
//!
//! Allows the SW runtime to dispatch RPC requests to a standard
//! guest WASM (built with the `entry!` macro FFI protocol) loaded
//! separately via `WebAssembly.instantiate`.
//!
//! ## Protocol
//!
//! The bridge encodes each dispatch as an `AbiFrame(op=GUEST_HANDLE)` containing
//! a `GuestHandleV1` with the `RpcEnvelope`. The JS callback is responsible for
//! copying the frame bytes into the guest WASM linear memory, calling `actr_handle`,
//! and returning the `AbiReply` bytes.
//!
//! ## Outbound host invocations (JSPI)
//!
//! When a guest WASM calls `actr_host_invoke` (e.g. for discover or call_raw),
//! the JS host routes the ABI frame to [`guest_host_invoke_async`] which
//! decodes the operation, performs it through the runtime context, and returns
//! an `AbiReply`. The current `RuntimeContext` is stored in `GUEST_CTX` for
//! the duration of each dispatch so `guest_host_invoke_async` can access it.

use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use actr_framework::guest::abi;
use actr_protocol::RpcEnvelope;
use actr_protocol::prost::Message as ProstMessage;
use bytes::Bytes;

use crate::context::RuntimeContext;
use crate::web_context::WebContext;
use crate::workload::{ServiceHandlerFn, WasmWorkload};

// Thread-local storage for the current RuntimeContext during guest dispatch.
// Set before calling the JS dispatch function, cleared after.
thread_local! {
    static GUEST_CTX: RefCell<Option<Rc<RuntimeContext>>> = const { RefCell::new(None) };
}

/// Encode an `InitPayloadV1` for guest WASM initialization.
///
/// Returns protobuf-encoded bytes that can be passed to the guest's `actr_init`.
#[wasm_bindgen]
pub fn encode_guest_init_payload(actr_type: &str, realm_id: u32) -> Vec<u8> {
    let init = abi::InitPayloadV1 {
        version: abi::version::V1,
        actr_type: actr_type.to_string(),
        credential: vec![],
        actor_id: vec![],
        realm_id,
    };
    init.encode_to_vec()
}

/// Register a guest workload backed by a JS dispatch function.
///
/// `dispatch_fn` signature (JS):
///   `(abiFrameBytes: Uint8Array) => Uint8Array | Promise<Uint8Array>`
///
///   - **Input**: protobuf-encoded `AbiFrame` with `op = GUEST_HANDLE`
///   - **Output**: protobuf-encoded `AbiReply` (sync or async via JSPI)
///
/// This enables the SW runtime to dispatch RPC requests to a standard
/// guest WASM (built with `entry!` macro) loaded separately via
/// `WebAssembly.instantiate`.
#[wasm_bindgen]
pub fn register_guest_workload(dispatch_fn: js_sys::Function) {
    let handler: ServiceHandlerFn = Rc::new(
        move |route_key: &str, body: &[u8], ctx: Rc<RuntimeContext>| {
            let dispatch_fn = dispatch_fn.clone();
            let route_key = route_key.to_string();
            let body = body.to_vec();

            Box::pin(async move {
                log::info!(
                    "[GuestBridge] Dispatch called: route_key={} body_len={}",
                    route_key,
                    body.len()
                );

                // Build RpcEnvelope containing the request
                let envelope = RpcEnvelope {
                    route_key: route_key.clone(),
                    payload: Some(Bytes::from(body)),
                    error: None,
                    traceparent: None,
                    tracestate: None,
                    request_id: ctx.request_id().to_string(),
                    metadata: vec![],
                    timeout_ms: 30000,
                };
                let envelope_bytes = envelope.encode_to_vec();

                // Build GuestHandleV1
                let handle = abi::GuestHandleV1 {
                    ctx: abi::InvocationContextV1 {
                        self_id: ctx.self_id().clone(),
                        caller_id: ctx.caller_id().cloned(),
                        request_id: ctx.request_id().to_string(),
                    },
                    rpc_envelope: envelope_bytes,
                };
                let handle_bytes = handle.encode_to_vec();

                // Build AbiFrame
                let frame = abi::AbiFrame {
                    abi_version: abi::version::V1,
                    op: abi::op::GUEST_HANDLE,
                    payload: handle_bytes,
                };
                let frame_bytes = frame.encode_to_vec();

                // Store the RuntimeContext for guest_host_invoke_async
                GUEST_CTX.with(|cell| cell.replace(Some(ctx.clone())));

                // Call JS dispatch function
                let js_bytes = js_sys::Uint8Array::from(&frame_bytes[..]);
                log::info!(
                    "[GuestBridge] Calling JS dispatch_fn with {} bytes",
                    frame_bytes.len()
                );
                let result = dispatch_fn.call1(&JsValue::NULL, &js_bytes).map_err(|e| {
                    GUEST_CTX.with(|cell| cell.replace(None));
                    format!("Guest dispatch failed: {:?}", e)
                })?;

                log::info!(
                    "[GuestBridge] JS dispatch_fn returned, result type: is_undefined={} is_null={}",
                    result.is_undefined(),
                    result.is_null()
                );

                if result.is_null() || result.is_undefined() {
                    GUEST_CTX.with(|cell| cell.replace(None));
                    return Err("Guest returned null/undefined".to_string());
                }

                // Handle both sync (Uint8Array) and async (Promise<Uint8Array>) returns.
                // When the guest uses JSPI for outbound calls, actr_handle returns a Promise.
                let reply_arr = if result.is_instance_of::<js_sys::Promise>() {
                    log::info!("[GuestBridge] Awaiting Promise from guest dispatch");
                    let promise = js_sys::Promise::from(result);
                    let resolved = wasm_bindgen_futures::JsFuture::from(promise)
                        .await
                        .map_err(|e| {
                            GUEST_CTX.with(|cell| cell.replace(None));
                            format!("Guest async dispatch failed: {:?}", e)
                        })?;
                    resolved.dyn_into::<js_sys::Uint8Array>().map_err(|e| {
                        GUEST_CTX.with(|cell| cell.replace(None));
                        format!("Guest async dispatch returned non-Uint8Array: {:?}", e)
                    })?
                } else {
                    result.dyn_into::<js_sys::Uint8Array>().map_err(|e| {
                        GUEST_CTX.with(|cell| cell.replace(None));
                        format!("Guest dispatch returned non-Uint8Array: {:?}", e)
                    })?
                };

                // Clear the context now that dispatch is complete
                GUEST_CTX.with(|cell| cell.replace(None));

                let reply_vec = reply_arr.to_vec();
                log::info!("[GuestBridge] Reply bytes: {}", reply_vec.len());
                let reply: abi::AbiReply = abi::AbiReply::decode(&*reply_vec)
                    .map_err(|e| format!("Failed to decode AbiReply: {}", e))?;

                log::info!(
                    "[GuestBridge] AbiReply status={} payload_len={}",
                    reply.status,
                    reply.payload.len()
                );

                if reply.status != abi::code::SUCCESS {
                    let error_msg = if reply.payload.is_empty() {
                        format!("Guest error (status={})", reply.status)
                    } else {
                        String::from_utf8(reply.payload)
                            .unwrap_or_else(|_| format!("Guest error (status={})", reply.status))
                    };
                    return Err(error_msg);
                }

                Ok(reply.payload)
            }) as Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>>>>
        },
    );

    crate::register_workload(WasmWorkload::new(handler));
    log::info!("[SW] Guest workload registered via JS bridge");
}

/// Handle a guest's outbound host invocation asynchronously.
///
/// Called from JS when the guest WASM invokes `actr_host_invoke`.
/// The current `RuntimeContext` must be available in `GUEST_CTX`
/// (set by `register_guest_workload` during dispatch).
///
/// Supports:
/// - `HOST_DISCOVER` (op=4): discover a target actor by `ActrType`
/// - `HOST_CALL_RAW` (op=3): raw RPC call to a target actor
/// - `HOST_CALL` (op=1): typed RPC call to a destination
///
/// Returns protobuf-encoded `AbiReply` bytes.
#[wasm_bindgen]
pub async fn guest_host_invoke_async(frame_bytes: &[u8]) -> Result<Vec<u8>, JsValue> {
    let ctx = GUEST_CTX
        .with(|cell| cell.borrow().clone())
        .ok_or_else(|| JsValue::from_str("No guest context available for host invoke"))?;

    let frame: abi::AbiFrame = abi::AbiFrame::decode(frame_bytes)
        .map_err(|e| JsValue::from_str(&format!("Failed to decode AbiFrame: {}", e)))?;

    log::info!(
        "[GuestBridge] host_invoke: op={} payload_len={}",
        frame.op,
        frame.payload.len()
    );

    match frame.op {
        abi::op::HOST_DISCOVER => {
            let discover: abi::HostDiscoverV1 =
                abi::HostDiscoverV1::decode(frame.payload.as_slice())
                    .map_err(|e| JsValue::from_str(&format!("decode HostDiscoverV1: {}", e)))?;

            log::info!(
                "[GuestBridge] HOST_DISCOVER: {}:{}:{}",
                discover.target_type.manufacturer,
                discover.target_type.name,
                discover.target_type.version
            );

            match ctx.discover(&discover.target_type).await {
                Ok(actor_id) => {
                    let reply_payload = actor_id.encode_to_vec();
                    let reply = abi::AbiReply {
                        abi_version: abi::version::V1,
                        status: abi::code::SUCCESS,
                        payload: reply_payload,
                    };
                    Ok(reply.encode_to_vec())
                }
                Err(e) => {
                    log::warn!("[GuestBridge] HOST_DISCOVER failed: {}", e);
                    let reply = abi::AbiReply {
                        abi_version: abi::version::V1,
                        status: abi::code::GENERIC_ERROR,
                        payload: e.to_string().into_bytes(),
                    };
                    Ok(reply.encode_to_vec())
                }
            }
        }
        abi::op::HOST_CALL_RAW => {
            let call_raw: abi::HostCallRawV1 = abi::HostCallRawV1::decode(frame.payload.as_slice())
                .map_err(|e| JsValue::from_str(&format!("decode HostCallRawV1: {}", e)))?;

            log::info!(
                "[GuestBridge] HOST_CALL_RAW: route_key={} payload_len={}",
                call_raw.route_key,
                call_raw.payload.len()
            );

            match ctx
                .call_raw(
                    &call_raw.target,
                    &call_raw.route_key,
                    &call_raw.payload,
                    30000,
                )
                .await
            {
                Ok(response) => {
                    let reply = abi::AbiReply {
                        abi_version: abi::version::V1,
                        status: abi::code::SUCCESS,
                        payload: response,
                    };
                    Ok(reply.encode_to_vec())
                }
                Err(e) => {
                    log::warn!("[GuestBridge] HOST_CALL_RAW failed: {}", e);
                    let reply = abi::AbiReply {
                        abi_version: abi::version::V1,
                        status: abi::code::GENERIC_ERROR,
                        payload: e.to_string().into_bytes(),
                    };
                    Ok(reply.encode_to_vec())
                }
            }
        }
        abi::op::HOST_CALL => {
            let host_call: abi::HostCallV1 = abi::HostCallV1::decode(frame.payload.as_slice())
                .map_err(|e| JsValue::from_str(&format!("decode HostCallV1: {}", e)))?;

            log::info!(
                "[GuestBridge] HOST_CALL: route_key={} payload_len={}",
                host_call.route_key,
                host_call.payload.len()
            );

            // Resolve the destination to a target ActrId
            let dest = host_call
                .dest
                .try_into_dest()
                .map_err(|e| JsValue::from_str(&format!("invalid dest: {}", e)))?;

            let target_id = match &dest {
                actr_framework::Dest::Actor(id) => id.clone(),
                _ => {
                    let reply = abi::AbiReply {
                        abi_version: abi::version::V1,
                        status: abi::code::UNSUPPORTED_OP,
                        payload: b"HOST_CALL only supports Actor destination in web runtime"
                            .to_vec(),
                    };
                    return Ok(reply.encode_to_vec());
                }
            };

            match ctx
                .call_raw(&target_id, &host_call.route_key, &host_call.payload, 30000)
                .await
            {
                Ok(response) => {
                    let reply = abi::AbiReply {
                        abi_version: abi::version::V1,
                        status: abi::code::SUCCESS,
                        payload: response,
                    };
                    Ok(reply.encode_to_vec())
                }
                Err(e) => {
                    log::warn!("[GuestBridge] HOST_CALL failed: {}", e);
                    let reply = abi::AbiReply {
                        abi_version: abi::version::V1,
                        status: abi::code::GENERIC_ERROR,
                        payload: e.to_string().into_bytes(),
                    };
                    Ok(reply.encode_to_vec())
                }
            }
        }
        _ => {
            log::warn!("[GuestBridge] Unsupported host invoke op: {}", frame.op);
            let reply = abi::AbiReply {
                abi_version: abi::version::V1,
                status: abi::code::UNSUPPORTED_OP,
                payload: format!("Unsupported op: {}", frame.op).into_bytes(),
            };
            Ok(reply.encode_to_vec())
        }
    }
}
