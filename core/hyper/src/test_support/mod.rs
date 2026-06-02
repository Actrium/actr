//! Shared integration-test helpers enabled by the `test-utils` feature.
//!
//! These modules are used by `core/hyper/tests/*` and live under the library
//! so their public APIs are treated as externally reachable rather than dead
//! code inside each individual integration test crate.

#[cfg(feature = "wasm-engine")]
use crate::HostOperationResult;
use crate::{BinaryKind, Hyper, WorkloadPackage};
#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
use crate::{HostAbiFn, InvocationContext};
#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
use actr_framework::guest::dynclib_abi::InitPayloadV1;
use actr_pack::PackageManifest;
#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
use actr_protocol::ActrId;

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

pub use crate::transport::lane::{
    WebRtcFragmentSendEvent, WebRtcFragmentSendHook, WebRtcFragmentSendHookGuard,
    install_webrtc_fragment_send_hook_for_test,
};

/// Assert whether an attached node has the runtime hook observer installed.
///
/// Package attach uses this observer to bridge observation hooks into Wasm /
/// DynClib guests; linked attach installs its own linked observer.
pub fn attached_node_has_hook_observer(node: &crate::Node<crate::Attached>) -> bool {
    node.attachment
        .as_ref()
        .expect("Node<Attached> without attachment")
        .node
        .hook_observer
        .is_some()
}

/// Public test-facing mirror of package observation hook events.
#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
#[derive(Debug, Clone)]
pub enum TestPackageHookEvent {
    SignalingConnecting,
    SignalingConnected,
    SignalingDisconnected,
    WebSocketConnecting { peer: ActrId },
    WebSocketConnected { peer: ActrId },
    WebSocketDisconnected { peer: ActrId },
    WebRtcConnecting { peer: ActrId },
    WebRtcConnected { peer: ActrId, relayed: bool },
    WebRtcDisconnected { peer: ActrId },
    CredentialRenewed { new_expiry: std::time::SystemTime },
    CredentialExpiring { new_expiry: std::time::SystemTime },
    MailboxBackpressure { queue_len: usize, threshold: usize },
}

#[cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]
impl From<TestPackageHookEvent> for crate::workload::PackageHookEvent {
    fn from(event: TestPackageHookEvent) -> Self {
        match event {
            TestPackageHookEvent::SignalingConnecting => Self::SignalingConnecting,
            TestPackageHookEvent::SignalingConnected => Self::SignalingConnected,
            TestPackageHookEvent::SignalingDisconnected => Self::SignalingDisconnected,
            TestPackageHookEvent::WebSocketConnecting { peer } => {
                Self::WebSocketConnecting(actr_framework::PeerEvent {
                    peer,
                    relayed: None,
                })
            }
            TestPackageHookEvent::WebSocketConnected { peer } => {
                Self::WebSocketConnected(actr_framework::PeerEvent {
                    peer,
                    relayed: None,
                })
            }
            TestPackageHookEvent::WebSocketDisconnected { peer } => {
                Self::WebSocketDisconnected(actr_framework::PeerEvent {
                    peer,
                    relayed: None,
                })
            }
            TestPackageHookEvent::WebRtcConnecting { peer } => {
                Self::WebRtcConnecting(actr_framework::PeerEvent {
                    peer,
                    relayed: None,
                })
            }
            TestPackageHookEvent::WebRtcConnected { peer, relayed } => {
                Self::WebRtcConnected(actr_framework::PeerEvent {
                    peer,
                    relayed: Some(relayed),
                })
            }
            TestPackageHookEvent::WebRtcDisconnected { peer } => {
                Self::WebRtcDisconnected(actr_framework::PeerEvent {
                    peer,
                    relayed: None,
                })
            }
            TestPackageHookEvent::CredentialRenewed { new_expiry } => {
                Self::CredentialRenewed(actr_framework::CredentialEvent { new_expiry })
            }
            TestPackageHookEvent::CredentialExpiring { new_expiry } => {
                Self::CredentialExpiring(actr_framework::CredentialEvent { new_expiry })
            }
            TestPackageHookEvent::MailboxBackpressure {
                queue_len,
                threshold,
            } => Self::MailboxBackpressure(actr_framework::BackpressureEvent {
                queue_len,
                threshold,
            }),
        }
    }
}

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
        let ctx = InvocationContext {
            self_id: actr_protocol::ActrId::default(),
            caller_id: None,
            request_id: "test:on_start".to_string(),
        };
        let host_abi: HostAbiFn =
            std::sync::Arc::new(|_| Box::pin(async { HostOperationResult::Done }));
        self.inner.call_on_start(ctx, &host_abi).await
    }

    pub async fn call_hook_event(
        &mut self,
        event: TestPackageHookEvent,
        ctx: InvocationContext,
        host_abi: &HostAbiFn,
    ) -> Result<(), crate::wasm::WasmError> {
        self.inner
            .call_hook_event(event.into(), ctx, host_abi)
            .await
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

    pub async fn call_hook_event(
        &mut self,
        event: TestPackageHookEvent,
        ctx: InvocationContext,
        call_executor: &HostAbiFn,
    ) -> Result<(), crate::dynclib::DynclibError> {
        self.inner
            .call_hook_event(event.into(), ctx, call_executor)
            .await
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
