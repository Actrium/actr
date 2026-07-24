//! Frozen-package regression guard for the async-lift rebuild diagnostic.
//!
//! `fixtures/legacy_asynclift_guest.wasm` is a real artifact built by the
//! wit-bindgen 0.57.1 async-lift pipeline. Wasmtime rejects its `async`
//! canonical option because the old `actr:workload@0.1.0` WIT functions are
//! synchronous. The fixture is intentionally frozen: reproducing these bytes
//! would require retaining the obsolete SDK and linker pipeline.

#![cfg(feature = "wasm-engine")]

use actr_hyper::wasm::{WasmError, WasmHost};

const LEGACY_ASYNCLIFT_GUEST: &[u8] = include_bytes!("fixtures/legacy_asynclift_guest.wasm");

#[test]
fn legacy_asynclift_package_is_rejected_with_rebuild_hint() {
    let error = WasmHost::compile(LEGACY_ASYNCLIFT_GUEST)
        .expect_err("legacy async-lift component must be rejected by wasmtime");

    let WasmError::LoadFailed(message) = &error else {
        panic!("expected WasmError::LoadFailed, got: {error:?}");
    };

    assert!(
        message.contains("old SDK") && message.contains("Rebuild"),
        "rejection should carry actionable rebuild guidance, got: {message}"
    );
    assert!(
        message.contains("async` canonical option requires an async function type"),
        "rejection should preserve the underlying wasmtime cause, got: {message}"
    );
}
