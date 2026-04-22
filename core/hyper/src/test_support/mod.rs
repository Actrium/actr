//! Shared integration-test helpers enabled by the `test-utils` feature.
//!
//! These modules are used by `core/hyper/tests/*` and live under the library
//! so their public APIs are treated as externally reachable rather than dead
//! code inside each individual integration test crate.

use crate::{BinaryKind, Hyper, WorkloadPackage};
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
