//! Dynclib actor e2e integration tests
//!
//! Validates the full call chain:
//! host (DynclibHost/DynclibInstance) -> actr_handle -> DynclibContext::call_raw()
//!                                   -> vtable trampoline -> call_executor -> response
//!
//! # Test scenarios
//!
//! 1. Unknown route -> returns error
//! 2. Echo route (no outbound calls) -> returns payload as-is
//! 3. Double route (triggers vtable call trampoline) -> returns x*2
//! 4. Multiple dispatches -> verifies state isolation between calls

#![cfg(feature = "dynclib-engine")]

use std::path::{Path, PathBuf};
use std::process::Command;

use actr_hyper::dynclib::DynclibHost;
use actr_hyper::executor::{CallExecutorFn, DispatchContext, IoResult, PendingCall};
use actr_protocol::{ActrId, ActrType, Realm, RpcEnvelope, prost::Message as ProstMessage};

// ---- helpers ---------------------------------------------------------------

fn fixture_so_path() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/dynclib_actor_fixture");

    // Build the fixture cdylib
    let status = Command::new("cargo")
        .args(["build"])
        .current_dir(&fixture_dir)
        .status()
        .expect("failed to build dynclib fixture");
    assert!(status.success(), "dynclib fixture build failed");

    let target_dir = fixture_dir.join("target/debug");
    if cfg!(target_os = "linux") {
        target_dir.join("libdynclib_actor_fixture.so")
    } else if cfg!(target_os = "macos") {
        target_dir.join("libdynclib_actor_fixture.dylib")
    } else {
        target_dir.join("dynclib_actor_fixture.dll")
    }
}

fn make_envelope(route_key: &str, payload: Vec<u8>) -> Vec<u8> {
    let envelope = RpcEnvelope {
        route_key: route_key.to_string(),
        payload: Some(payload.into()),
        ..Default::default()
    };
    envelope.encode_to_vec()
}

fn test_actr_id() -> ActrId {
    ActrId {
        realm: Realm { realm_id: 1 },
        serial_number: 1,
        r#type: ActrType {
            manufacturer: "test".to_string(),
            name: "fixture".to_string(),
            version: "0.1.0".to_string(),
        },
    }
}

fn test_ctx() -> DispatchContext {
    DispatchContext {
        self_id: test_actr_id(),
        caller_id: None,
        request_id: "test-req-001".to_string(),
    }
}

fn noop_executor() -> CallExecutorFn {
    Box::new(|_pending| Box::pin(async { IoResult::Error(-1) }))
}

// ---- tests -----------------------------------------------------------------

/// Unknown route -> dispatch returns error
#[tokio::test]
#[ignore] // requires fixture compilation
async fn dynclib_unknown_route_returns_error() {
    let so_path = fixture_so_path();
    let host = DynclibHost::load(&so_path).expect("load SO");
    let mut instance = host.instantiate(b"{}").expect("instantiate");

    let req_bytes = make_envelope("unknown/route", vec![1, 0, 0, 0]);
    let executor = noop_executor();

    let result = instance.dispatch(&req_bytes, test_ctx(), &executor).await;

    assert!(result.is_err(), "unknown route should return error");
}

/// Echo route -> returns payload without outbound calls
#[tokio::test]
#[ignore]
async fn dynclib_echo_returns_payload() {
    let so_path = fixture_so_path();
    let host = DynclibHost::load(&so_path).expect("load SO");
    let mut instance = host.instantiate(b"{}").expect("instantiate");

    let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let req_bytes = make_envelope("test/echo", payload.clone());
    let executor = noop_executor();

    let result = instance
        .dispatch(&req_bytes, test_ctx(), &executor)
        .await
        .expect("echo dispatch failed");

    assert_eq!(result, payload, "echo should return payload as-is");
}

/// Double route -> triggers vtable call trampoline, returns x*2
#[tokio::test]
#[ignore]
async fn dynclib_double_dispatch() {
    let so_path = fixture_so_path();
    let host = DynclibHost::load(&so_path).expect("load SO");
    let mut instance = host.instantiate(b"{}").expect("instantiate");

    let x: i32 = 7;
    let req_bytes = make_envelope("test/double", x.to_le_bytes().to_vec());

    // call_executor: handle the vtable call from guest's ctx.call_raw()
    // DynclibContext::call_raw encodes Dest::Actor and routes through vtable.call,
    // which produces PendingCall::Call { route_key, dest_bytes, payload }.
    let executor: CallExecutorFn = Box::new(|pending| {
        Box::pin(async move {
            match pending {
                PendingCall::Call {
                    route_key, payload, ..
                } => {
                    assert_eq!(route_key, "test/double_impl", "route_key mismatch");
                    assert_eq!(payload.len(), 4, "payload should be 4 bytes");

                    let val = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    assert_eq!(val, 7, "guest should pass x=7");

                    // mock: return x * 2
                    let doubled = (val * 2).to_le_bytes().to_vec();
                    IoResult::Bytes(doubled)
                }
                other => panic!(
                    "expected PendingCall::Call, got {:?}",
                    std::mem::discriminant(&other)
                ),
            }
        })
    });

    let result = instance
        .dispatch(&req_bytes, test_ctx(), &executor)
        .await
        .expect("double dispatch failed");

    assert_eq!(result.len(), 4, "response should be 4 bytes");
    let resp_val = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
    assert_eq!(resp_val, 14, "response should be 7 * 2 = 14");
}

/// Multiple dispatches -> verifies state does not leak between calls
#[tokio::test]
#[ignore]
async fn dynclib_multiple_dispatches() {
    let so_path = fixture_so_path();
    let host = DynclibHost::load(&so_path).expect("load SO");
    let mut instance = host.instantiate(b"{}").expect("instantiate");

    for x in [1i32, 5, 42, 100] {
        let req_bytes = make_envelope("test/double", x.to_le_bytes().to_vec());

        let executor: CallExecutorFn = Box::new(|pending| {
            Box::pin(async move {
                match pending {
                    PendingCall::Call { payload, .. } => {
                        let val =
                            i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                        IoResult::Bytes((val * 2).to_le_bytes().to_vec())
                    }
                    _ => IoResult::Error(-1),
                }
            })
        });

        let result = instance
            .dispatch(&req_bytes, test_ctx(), &executor)
            .await
            .expect("dispatch failed");

        let resp_val = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
        assert_eq!(resp_val, x * 2, "dispatch({x}) should return {}", x * 2);
    }
}
