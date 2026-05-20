//! WasmHost minimal failure-path coverage.
//!
//! Component dispatch behaviour is covered by `component_model_dispatch.rs`.
//! This file keeps only the invalid-binary failure path for `WasmHost::compile`.

#![cfg(feature = "wasm-engine")]

use actr_hyper::wasm::WasmHost;

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
