//! DynclibHost unit-level tests
//!
//! Tests basic host operations:
//! - Loading a non-existent library -> error
//! - Loading and instantiating a valid SO -> success
//! - Basic request handling through the loaded instance

#![cfg(feature = "dynclib-engine")]

use actr_framework::guest::dynclib_abi::{InitPayloadV1, version};
use std::path::{Path, PathBuf};
use std::process::Command;

use actr_hyper::dynclib::{DynclibError, DynclibHost};
use actr_hyper::test_support::instantiate_dynclib_workload;
use actr_hyper::workload::{HostAbiFn, HostOperation, HostOperationResult, InvocationContext};
use actr_protocol::{ActrId, ActrType, Realm, RpcEnvelope, prost::Message as ProstMessage};

// ---- helpers ---------------------------------------------------------------

fn build_fixture() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/dynclib_actor_fixture");

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

fn test_ctx() -> InvocationContext {
    InvocationContext {
        self_id: test_actr_id(),
        caller_id: None,
        request_id: "test".to_string(),
    }
}

fn test_config() -> InitPayloadV1 {
    InitPayloadV1 {
        version: version::V1,
        actr_type: "test:fixture:0.1.0".to_string(),
        credential: Vec::new(),
        actor_id: Vec::new(),
        realm_id: 1,
    }
}

// ---- tests -----------------------------------------------------------------

/// Loading a non-existent library path should return LoadFailed
#[test]
fn test_load_nonexistent_library() {
    let result = DynclibHost::load("/tmp/nonexistent_library_xyz.so");
    assert!(result.is_err(), "loading non-existent library should fail");
    let err = result.unwrap_err();
    assert!(
        matches!(err, DynclibError::LoadFailed(_)),
        "error should be LoadFailed, got: {err:?}"
    );
}

/// Loading and instantiating a valid SO should succeed
#[test]
#[ignore]
fn test_load_and_instantiate() {
    let so_path = build_fixture();
    let host = DynclibHost::load(&so_path).expect("load should succeed");
    let _instance =
        instantiate_dynclib_workload(host, &test_config()).expect("instantiate should succeed");
}

/// Basic echo dispatch through loaded instance
#[tokio::test]
#[ignore]
async fn test_basic_echo_dispatch() {
    let so_path = build_fixture();
    let host = DynclibHost::load(&so_path).expect("load");
    let mut instance = instantiate_dynclib_workload(host, &test_config()).expect("instantiate");

    let payload = b"hello dynclib".to_vec();
    let req_bytes = make_envelope("test/echo", payload.clone());

    let executor: HostAbiFn =
        std::sync::Arc::new(|_| Box::pin(async { HostOperationResult::Error(-1) }));

    let result = instance
        .handle(&req_bytes, test_ctx(), &executor)
        .await
        .expect("echo dispatch should succeed");

    assert_eq!(result, payload, "echo should return input payload");
}

/// Basic double dispatch through loaded instance
#[tokio::test]
#[ignore]
async fn test_basic_double_dispatch() {
    let so_path = build_fixture();
    let host = DynclibHost::load(&so_path).expect("load");
    let mut instance = instantiate_dynclib_workload(host, &test_config()).expect("instantiate");

    let x: i32 = 21;
    let req_bytes = make_envelope("test/double", x.to_le_bytes().to_vec());

    let executor: HostAbiFn = std::sync::Arc::new(|pending| {
        Box::pin(async move {
            match pending {
                HostOperation::Call(req) => {
                    let val = i32::from_le_bytes([
                        req.payload[0],
                        req.payload[1],
                        req.payload[2],
                        req.payload[3],
                    ]);
                    HostOperationResult::Bytes((val * 2).to_le_bytes().to_vec())
                }
                _ => HostOperationResult::Error(-1),
            }
        })
    });

    let result = instance
        .handle(&req_bytes, test_ctx(), &executor)
        .await
        .expect("double dispatch should succeed");

    let resp_val = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
    assert_eq!(resp_val, 42, "21 * 2 should be 42");
}
