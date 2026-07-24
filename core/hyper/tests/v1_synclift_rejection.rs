//! Frozen-package regression guard for explicit V1 world rejection.
//!
//! `fixtures/v1_synclift_guest.wasm` is a genuine, loadable
//! `actr:workload@0.1.0` component built before the SDK switched to V2. Unlike
//! the synthetic WAT unit test, this verifies that a shipped V1 artifact reaches
//! world probing and receives the current rebuild diagnostic.

#![cfg(feature = "wasm-engine")]

use actr_hyper::test_support::instantiate_wasm_workload;
use actr_hyper::wasm::{WasmError, WasmHost};

const V1_SYNCLIFT_GUEST: &[u8] = include_bytes!("fixtures/v1_synclift_guest.wasm");

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_synclift_package_is_rejected_with_rebuild_hint() {
    let host = WasmHost::compile(V1_SYNCLIFT_GUEST)
        .expect("frozen V1 sync-lift component must compile before world probing");
    let error = instantiate_wasm_workload(&host)
        .await
        .expect_err("retired V1 workload world must be rejected");

    let WasmError::LoadFailed(message) = &error else {
        panic!("expected WasmError::LoadFailed, got: {error:?}");
    };
    assert!(
        message.contains("retired actr:workload@0.1.0")
            && message.contains("rebuild the package with the current SDK"),
        "V1 rejection should carry actionable rebuild guidance, got: {message}"
    );
}
