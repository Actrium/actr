//! WASM actor end-to-end integration tests
//!
//! Validates the complete call chain:
//! host (WasmHost/WasmInstance) -> actr_handle -> WasmContext::call_raw()
//!                            -> asyncify unwind -> drive loop -> mock IO
//!                            -> asyncify rewind -> return response
//!
//! # Test scenarios
//!
//! 1. **Simple call without IO**: handler returns response directly, no asyncify triggered
//! 2. **call_raw triggering asyncify**: handler calls `ctx.call_raw()`,
//!    host mock returns 2x payload value, verifies correct asyncify suspend/resume

#![cfg(feature = "wasm-engine")]

include!("wasm_actor_fixture.rs");

use actr_hyper::wasm::{
    DispatchContext, IoResult, PendingCall, WasmActorConfig, WasmError, WasmHost,
};
use actr_protocol::{ActrId, RpcEnvelope, prost::Message as ProstMessage};

// ─── Helper: build test RpcEnvelope bytes ──────────────────────────────────────

fn make_envelope(route_key: &str, payload: Vec<u8>) -> Vec<u8> {
    let envelope = RpcEnvelope {
        route_key: route_key.to_string(),
        payload: Some(payload.into()),
        ..Default::default()
    };
    envelope.encode_to_vec()
}

// ─── Helper: build DispatchContext ──────────────────────────────────────────────

fn test_ctx() -> DispatchContext {
    DispatchContext {
        self_id: ActrId::default(),
        caller_id: None,
        request_id: "test-req-001".to_string(),
    }
}

// ─── Test 1: unknown route returns error ────────────────────────────────────────

#[tokio::test]
async fn wasm_actor_unknown_route_returns_error() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile failed");
    let mut instance = host.instantiate().expect("instantiate failed");

    let config = WasmActorConfig {
        actr_type: "test:double:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&config).expect("init failed");

    let req_bytes = make_envelope("unknown/route", vec![1, 0, 0, 0]);

    // This route does not exist; should return error (dispatch returns Err, actr_handle returns HANDLE_FAILED)
    let result = instance
        .dispatch(&req_bytes, test_ctx(), |_pending| async { IoResult::Done })
        .await;

    // Should fail (WASM returns HANDLE_FAILED error code)
    assert!(result.is_err(), "unknown route should return error");
    tracing::info!("unknown route error verified");
}

// ─── Test 1b: repeated init is rejected ───────────────────────────────────────

#[test]
fn wasm_actor_repeated_init_returns_error() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile failed");
    let mut instance = host.instantiate().expect("instantiate failed");

    let config = WasmActorConfig {
        actr_type: "test:double:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };

    instance.init(&config).expect("first init failed");

    let err = instance
        .init(&config)
        .expect_err("second init should fail for the same guest instance");

    match err {
        WasmError::InitFailed(message) => {
            assert!(
                message.contains("error code -2") && message.contains("initialization failed"),
                "expected INIT_FAILED error message, got: {message}"
            );
        }
        other => panic!("expected InitFailed, got {other:?}"),
    }
}

// ─── Test 2: call_raw triggering asyncify ──────────────────────────────────

#[tokio::test]
async fn wasm_actor_call_raw_triggers_asyncify() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile failed");
    let mut instance = host.instantiate().expect("instantiate failed");

    let config = WasmActorConfig {
        actr_type: "test:double:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&config).expect("init failed");

    // Send x = 7 (4-byte little-endian i32)
    let x: i32 = 7;
    let req_bytes = make_envelope("test/double", x.to_le_bytes().to_vec());

    // call_executor: handles call_raw requests from WASM
    // Expects route_key = "test/double_impl", payload = x's bytes
    // Returns x * 2 bytes
    let result = instance
        .dispatch(&req_bytes, test_ctx(), |pending| async move {
            match pending {
                PendingCall::CallRaw {
                    route_key, payload, ..
                } => {
                    assert_eq!(route_key, "test/double_impl", "route_key mismatch");
                    assert_eq!(payload.len(), 4, "payload should be 4 bytes");

                    let val = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    assert_eq!(val, 7, "WASM should pass x=7");

                    // mock IO: return x * 2
                    let doubled = (val * 2).to_le_bytes().to_vec();
                    tracing::info!(val, doubled = val * 2, "mock IO done");
                    IoResult::Bytes(doubled)
                }
                other => panic!(
                    "expected PendingCall::CallRaw, got {:?}",
                    std::mem::discriminant(&other)
                ),
            }
        })
        .await
        .expect("dispatch failed");

    // Response should be x * 2 = 14
    assert_eq!(result.len(), 4, "response should be 4 bytes");
    let resp_val = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
    assert_eq!(resp_val, 14, "response value should be 7 * 2 = 14");

    tracing::info!(resp_val, "WASM actor asyncify call_raw e2e verified");
}

// ─── Test 3: multiple dispatches, asyncify data buffer correctly resets between calls ─────────

#[tokio::test]
async fn wasm_actor_multiple_dispatches() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile failed");
    let mut instance = host.instantiate().expect("instantiate failed");

    let config = WasmActorConfig {
        actr_type: "test:double:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&config).expect("init failed");

    for x in [1i32, 5, 42, 100] {
        let req_bytes = make_envelope("test/double", x.to_le_bytes().to_vec());

        let result = instance
            .dispatch(&req_bytes, test_ctx(), |pending| async move {
                match pending {
                    PendingCall::CallRaw { payload, .. } => {
                        let val =
                            i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                        IoResult::Bytes((val * 2).to_le_bytes().to_vec())
                    }
                    _ => IoResult::Error(-1),
                }
            })
            .await
            .expect("dispatch failed");

        let resp_val = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
        assert_eq!(resp_val, x * 2, "dispatch({x}) should return {}", x * 2);
    }

    tracing::info!("multiple dispatch verified");
}
