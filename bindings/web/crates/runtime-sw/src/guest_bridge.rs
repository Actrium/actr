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
///   `(abiFrameBytes: Uint8Array) => Uint8Array`
///
///   - **Input**: protobuf-encoded `AbiFrame` with `op = GUEST_HANDLE`
///   - **Output**: protobuf-encoded `AbiReply`
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

                // Call JS dispatch function
                let js_bytes = js_sys::Uint8Array::from(&frame_bytes[..]);
                log::info!(
                    "[GuestBridge] Calling JS dispatch_fn with {} bytes",
                    frame_bytes.len()
                );
                let result = dispatch_fn
                    .call1(&JsValue::NULL, &js_bytes)
                    .map_err(|e| format!("Guest dispatch failed: {:?}", e))?;

                log::info!(
                    "[GuestBridge] JS dispatch_fn returned, result type: is_undefined={} is_null={}",
                    result.is_undefined(),
                    result.is_null()
                );

                if result.is_null() || result.is_undefined() {
                    return Err("Guest returned null/undefined".to_string());
                }

                // Decode AbiReply — result is a Uint8Array returned by guestDispatch
                // Use unchecked cast: we trust guestDispatch to return a Uint8Array
                let reply_arr = result
                    .dyn_into::<js_sys::Uint8Array>()
                    .map_err(|e| format!("Guest dispatch returned non-Uint8Array: {:?}", e))?;
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
