//! WasmHost minimal failure-path coverage.
//!
//! Phase 1 Commit 6 removed the five `#[ignore]` stubs that stood in for
//! the pre-Phase-1 asyncify ABI's behavioural coverage. Each stub's
//! intent is now served by a real test in
//! `component_model_dispatch.rs`:
//!
//! | Old stub                          | Replacement                                 |
//! | --------------------------------- | ------------------------------------------- |
//! | `wasm_host_compile_and_echo`      | `component_model_basic_echo_round_trip`     |
//! | `wasm_host_multiple_dispatches`   | `component_model_per_call_overhead`         |
//! | `wasm_host_missing_exports`       | covered by `WasmHost::compile`'s Component  |
//! |                                   | validation (legacy core wasm now rejected)  |
//! | `wasm_host_empty_dispatch`        | folded into the per-call-overhead loop      |
//! | `wasm_host_large_dispatch`        | not Component-ABI-specific, dropped as      |
//! |                                   | redundant with the 1000-dispatch benchmark  |
//!
//! Only the invalid-binary failure path is retained here because it
//! exercises `WasmHost::compile`'s reporting independently of the guest
//! contract.

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
