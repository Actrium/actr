//! Shared integration-test helpers enabled by the `test-utils` feature.
//!
//! These modules are used by `core/hyper/tests/*` and live under the library
//! so their public APIs are treated as externally reachable rather than dead
//! code inside each individual integration test crate.

use crate::{BinaryKind, Hyper, WorkloadPackage};
#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
use crate::{HostAbiFn, InvocationContext};
#[cfg(feature = "dynclib-engine")]
use actr_framework::guest::dynclib_abi::InitPayloadV1;
use actr_pack::PackageManifest;

#[path = "../../tests/common/harness.rs"]
pub mod harness;
#[path = "../../tests/common/signaling.rs"]
pub mod signaling;
#[path = "../../tests/common/utils.rs"]
pub mod utils;
#[path = "../../tests/common/vnet.rs"]
pub mod vnet;

pub use harness::{TestHarness, TestPeer};
pub use signaling::TestSignalingServer;
pub use utils::{
    create_credential_state_for_test, create_peer_with_vnet, create_peer_with_websocket,
    dummy_credential, make_actor_id, spawn_echo_responder, spawn_response_receiver,
};
pub use vnet::{VNetPair, create_vnet_pair};

/// Test-only summary of package loading results.
///
/// This keeps `LoadedWorkload` crate-private while preserving the assertions
/// integration tests care about: selected backend plus parsed manifest.
#[derive(Debug, Clone)]
pub struct LoadedWorkloadSummary {
    pub binary_kind: BinaryKind,
    manifest: PackageManifest,
}

impl LoadedWorkloadSummary {
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }
}

/// Verify a package, pick the execution backend, and return a test-facing
/// summary without exposing the runtime workload internals on the public API.
pub async fn inspect_workload_package(
    hyper: &Hyper,
    package: &WorkloadPackage,
) -> crate::error::HyperResult<LoadedWorkloadSummary> {
    let loaded = hyper.load_workload_package(package).await?;
    Ok(LoadedWorkloadSummary {
        binary_kind: loaded.binary_kind,
        manifest: loaded.verified.manifest,
    })
}

/// Test-only wrapper around Hyper's internal Component Model workload instance.
#[cfg(feature = "wasm-engine")]
#[derive(Debug)]
pub struct TestWasmWorkload {
    inner: crate::wasm::WasmWorkload,
}

#[cfg(feature = "wasm-engine")]
impl TestWasmWorkload {
    pub fn init(&mut self, init_payload: &InitPayloadV1) -> Result<(), crate::wasm::WasmError> {
        self.inner.init(init_payload)
    }

    pub async fn call_on_start(&mut self) -> Result<(), crate::wasm::WasmError> {
        self.inner.call_on_start().await
    }

    pub async fn handle(
        &mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> Result<Vec<u8>, crate::wasm::WasmError> {
        self.inner.handle(request_bytes, ctx, host_abi).await
    }
}

/// Instantiate a Component Model workload for integration tests without
/// exposing Hyper's internal runtime workload type on the public API.
#[cfg(feature = "wasm-engine")]
pub async fn instantiate_wasm_workload(
    host: &crate::wasm::WasmHost,
) -> Result<TestWasmWorkload, crate::wasm::WasmError> {
    Ok(TestWasmWorkload {
        inner: host.instantiate().await?,
    })
}

/// Test-only wrapper around Hyper's internal dynclib workload instance.
#[cfg(feature = "dynclib-engine")]
#[derive(Debug)]
pub struct TestDynclibWorkload {
    inner: crate::dynclib::DynClibWorkload,
}

#[cfg(feature = "dynclib-engine")]
impl TestDynclibWorkload {
    pub async fn handle(
        &mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        call_executor: &HostAbiFn,
    ) -> Result<Vec<u8>, crate::dynclib::DynclibError> {
        self.inner.handle(request_bytes, ctx, call_executor).await
    }
}

/// Instantiate a dynclib workload for integration tests while keeping
/// `DynclibInstance` crate-private.
#[cfg(feature = "dynclib-engine")]
pub fn instantiate_dynclib_workload(
    host: crate::dynclib::DynclibHost,
    init_payload: &InitPayloadV1,
) -> Result<TestDynclibWorkload, crate::dynclib::DynclibError> {
    let instance = host.instantiate(init_payload)?;
    Ok(TestDynclibWorkload {
        inner: crate::dynclib::DynClibWorkload::new(host, instance),
    })
}
