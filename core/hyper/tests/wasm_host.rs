//! Integration tests: WasmHost + WasmWorkload ABI verification

#![cfg(feature = "wasm-engine")]

use actr_framework::guest::abi::{InitPayloadV1, version};
use actr_hyper::wasm::WasmHost;
use actr_hyper::workload::{HostOperationResult, InvocationContext};
use actr_protocol::{ActrId, RpcEnvelope, prost::Message as ProstMessage};

include!("wasm_actor_fixture.rs");

fn make_envelope(route_key: &str, payload: Vec<u8>) -> Vec<u8> {
    let envelope = RpcEnvelope {
        route_key: route_key.to_string(),
        payload: Some(payload.into()),
        ..Default::default()
    };
    envelope.encode_to_vec()
}

/// Minimal valid init payload
fn test_config() -> InitPayloadV1 {
    InitPayloadV1 {
        version: version::V1,
        actr_type: "test:fixture:0.1.0".to_string(),
        credential: Vec::new(),
        actor_id: Vec::new(),
        realm_id: 1,
    }
}

/// Echo dispatch does not call host imports; host ABI will not be triggered.
fn noop_ctx() -> InvocationContext {
    InvocationContext {
        self_id: ActrId::default(),
        caller_id: None,
        request_id: "test".to_string(),
    }
}

// ─── Test cases ─────────────────────────────────────────────────────────────────

/// Scenario 1: normal flow -- compile, instantiate, init, dispatch (echo)
#[tokio::test]
async fn wasm_host_compile_and_echo() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile should succeed");
    let mut instance = host.instantiate().expect("instantiate should succeed");

    instance.init(&test_config()).expect("init should succeed");

    let request = b"hello, wasm!".to_vec();
    let req_bytes = make_envelope("test/echo", request.clone());
    let response = instance
        .handle(&req_bytes, noop_ctx(), |_| async {
            HostOperationResult::Done
        })
        .await
        .expect("dispatch should succeed");

    assert_eq!(
        response, request,
        "echo guest should return request data as-is"
    );
}

/// Scenario 2: multiple dispatches (verify bump allocator does not overflow)
#[tokio::test]
async fn wasm_host_multiple_dispatches() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    for i in 0u8..10 {
        let req = vec![i; 64];
        let req_bytes = make_envelope("test/echo", req.clone());
        let resp = instance
            .handle(&req_bytes, noop_ctx(), |_| async {
                HostOperationResult::Done
            })
            .await
            .expect("each dispatch should succeed");
        assert_eq!(resp, req, "dispatch #{i} should echo correctly");
    }
}

/// Scenario 3: invalid WASM binary -> WasmLoadFailed
#[test]
fn wasm_host_invalid_binary() {
    let bad_bytes = b"not a wasm file";
    let result = WasmHost::compile(bad_bytes);
    assert!(
        result.is_err(),
        "invalid WASM bytes should return error, got: {result:?}"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, actr_hyper::wasm::WasmError::LoadFailed(_)),
        "error type should be WasmLoadFailed, got: {err:?}"
    );
}

/// Scenario 4: WASM missing required exports -> WasmLoadFailed (at instantiation)
#[test]
fn wasm_host_missing_exports() {
    // Only memory, no actr_* functions
    let incomplete_wat = r#"
(module
  (memory (export "memory") 1)
)
"#;
    let wasm_bytes = wat::parse_str(incomplete_wat).unwrap();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let result = host.instantiate();

    assert!(
        result.is_err(),
        "missing exports should error at instantiation, got: {result:?}"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, actr_hyper::wasm::WasmError::LoadFailed(_)),
        "error type should be WasmLoadFailed, got: {err:?}"
    );
}

/// Scenario 5: empty request (0 bytes) -> dispatch should return empty response
#[tokio::test]
async fn wasm_host_empty_dispatch() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    let req_bytes = make_envelope("test/echo", Vec::new());
    let response = instance
        .handle(&req_bytes, noop_ctx(), |_| async {
            HostOperationResult::Done
        })
        .await
        .expect("empty request dispatch should succeed");
    assert!(
        response.is_empty(),
        "empty request should return empty response"
    );
}

/// Scenario 6: large request (16KB) -> verify memory operations correctness
#[tokio::test]
async fn wasm_host_large_dispatch() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    let large_req: Vec<u8> = (0..16384u16).map(|i| (i % 251) as u8).collect();
    let req_bytes = make_envelope("test/echo", large_req.clone());
    let resp = instance
        .handle(&req_bytes, noop_ctx(), |_| async {
            HostOperationResult::Done
        })
        .await
        .expect("large request dispatch should succeed");
    assert_eq!(
        resp, large_req,
        "large request echo content should match exactly"
    );
}
