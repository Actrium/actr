//! Integration tests: WasmHost + WasmInstance ABI verification
//!
//! Uses inline WAT (WebAssembly Text Format) to construct a minimal guest module:
//! - bump allocator (satisfies actr_alloc / actr_free interface)
//! - actr_init: returns SUCCESS directly
//! - actr_handle: echoes request back as-is (verifies memory read/write correctness)

#![cfg(feature = "wasm-engine")]

use actr_hyper::wasm::{DispatchContext, IoResult, WasmActorConfig, WasmHost};
use actr_protocol::ActrId;

// ─── Minimal guest WAT module ──────────────────────────────────────────────────

/// Returns the minimal WASM binary implementing the full actr ABI (echo guest)
///
/// Guest behavior:
/// - `actr_alloc`: bump allocator (starting from offset 4096)
/// - `actr_free`: no-op
/// - `actr_init`: does nothing, returns 0 (SUCCESS)
/// - `actr_handle`: echoes request data to a newly allocated response buffer
fn echo_guest_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"
(module
  ;; 2 pages of memory (128 KB), exported
  (memory (export "memory") 2)

  ;; heap start position (first 4096 bytes reserved for output buffer)
  (global $heap (mut i32) (i32.const 4096))

  ;; internal bump allocator
  (func $bump (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))

  ;; actr_alloc(size) -> ptr
  (func (export "actr_alloc") (param $n i32) (result i32)
    (call $bump (local.get $n)))

  ;; actr_free(ptr, size) -- no-op (bump allocator does not reclaim)
  (func (export "actr_free") (param $p i32) (param $n i32))

  ;; asyncify stub functions (echo guest does not trigger asyncify, but instantiate() requires exports)
  (func (export "asyncify_start_unwind") (param i32))
  (func (export "asyncify_stop_unwind"))
  (func (export "asyncify_start_rewind") (param i32))
  (func (export "asyncify_stop_rewind"))

  ;; actr_init(config_ptr, config_len) -> i32
  ;; This guest ignores config and returns 0 (SUCCESS) directly
  (func (export "actr_init") (param $p i32) (param $n i32) (result i32)
    (i32.const 0))

  ;; actr_handle(req_ptr, req_len, resp_ptr_out, resp_len_out) -> i32
  ;; Allocates response buffer equal to request length, copies request content (echo), writes output pointers
  (func (export "actr_handle")
    (param $req_ptr i32) (param $req_len i32)
    (param $resp_ptr_out i32) (param $resp_len_out i32)
    (result i32)
    (local $resp_ptr i32)

    ;; allocate response memory
    (local.set $resp_ptr (call $bump (local.get $req_len)))

    ;; copy request to response (echo)
    (memory.copy
      (local.get $resp_ptr)
      (local.get $req_ptr)
      (local.get $req_len))

    ;; write output area: resp_ptr_out and resp_len_out (little-endian i32)
    (i32.store (local.get $resp_ptr_out) (local.get $resp_ptr))
    (i32.store (local.get $resp_len_out) (local.get $req_len))

    ;; return SUCCESS
    (i32.const 0))
)
"#,
    )
    .expect("WAT parse failed")
}

/// Minimal valid `WasmActorConfig`
fn test_config() -> WasmActorConfig {
    WasmActorConfig {
        actr_type: "test-mfr:echo-actor:0.1.0".to_string(),
        credential_b64: "dGVzdA==".to_string(), // base64("test")
        actor_id_b64: "aWQ=".to_string(),       // base64("id")
        realm_id: 1,
    }
}

/// Echo guest does not call host imports; call_executor will not be triggered
fn noop_ctx() -> DispatchContext {
    DispatchContext {
        self_id: ActrId::default(),
        caller_id: None,
        request_id: "test".to_string(),
    }
}

// ─── Test cases ─────────────────────────────────────────────────────────────────

/// Scenario 1: normal flow -- compile, instantiate, init, dispatch (echo)
#[tokio::test]
async fn wasm_host_compile_and_echo() {
    let wasm_bytes = echo_guest_wasm();

    let host = WasmHost::compile(&wasm_bytes).expect("compile should succeed");
    let mut instance = host.instantiate().expect("instantiate should succeed");

    instance.init(&test_config()).expect("init should succeed");

    let request = b"hello, wasm!".to_vec();
    let response = instance
        .dispatch(&request, noop_ctx(), |_| async { IoResult::Done })
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
    let wasm_bytes = echo_guest_wasm();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    for i in 0u8..10 {
        let req = vec![i; 64];
        let resp = instance
            .dispatch(&req, noop_ctx(), |_| async { IoResult::Done })
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
    let wasm_bytes = echo_guest_wasm();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    let response = instance
        .dispatch(&[], noop_ctx(), |_| async { IoResult::Done })
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
    let wasm_bytes = echo_guest_wasm();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    let large_req: Vec<u8> = (0..16384u16).map(|i| (i % 251) as u8).collect();
    let resp = instance
        .dispatch(&large_req, noop_ctx(), |_| async { IoResult::Done })
        .await
        .expect("large request dispatch should succeed");
    assert_eq!(
        resp, large_req,
        "large request echo content should match exactly"
    );
}
