//! WasmHost + WasmWorkload ABI verification (Phase 1: pending port).
//!
//! These tests exercised the pre-Phase-1 core-wasm-module + handwritten
//! ptr/len ABI. Phase 1 Commit 2 rewrote the host around the Component
//! Model; the legacy fixtures no longer load. The test suite is
//! rewritten in Phase 1 Commit 6 against Component Model guests. The
//! stubs below keep the build green during the intervening commits.
//!
//! One test (`wasm_host_invalid_binary`) stays live because it only
//! exercises the failure path when invalid bytes are passed to
//! `WasmHost::compile` — that code path is independent of the ABI
//! generation and is worth keeping green through the migration.

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

#[test]
#[ignore = "Phase 1 Commit 6 rewrites these tests against Component Model guests"]
fn wasm_host_compile_and_echo() {}

#[test]
#[ignore = "Phase 1 Commit 6 rewrites these tests against Component Model guests"]
fn wasm_host_multiple_dispatches() {}

#[test]
#[ignore = "Phase 1 Commit 6 rewrites these tests against Component Model guests"]
fn wasm_host_missing_exports() {}

#[test]
#[ignore = "Phase 1 Commit 6 rewrites these tests against Component Model guests"]
fn wasm_host_empty_dispatch() {}

#[test]
#[ignore = "Phase 1 Commit 6 rewrites these tests against Component Model guests"]
fn wasm_host_large_dispatch() {}
