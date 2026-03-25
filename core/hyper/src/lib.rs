//! # actr-hyper
//!
//! Hyper — Actor platform layer + runtime infrastructure
//!
//! ## Positioning
//!
//! Hyper is the operating system for Actors: it defines boundaries (Sandbox), provides platform
//! primitives, and carries the full runtime infrastructure (transport, routing, lifecycle management).
//!
//! An Actor cannot open a database on its own, cannot hold its own private key, and cannot claim
//! to be a certain type — everything must go through Hyper's controlled interfaces.
//!
//! ## Responsibilities
//!
//! ### Platform Layer (formerly Hyper)
//!
//! - Package signature verification (binary_hash + MFR signature)
//! - Actor bootstrap (registers with AIS on behalf of the Actor, obtains credential)
//! - Storage namespace isolation (independent SQLite space per Actor)
//! - Cryptographic primitives (Ed25519 sign/verify, Actor does not hold raw private keys)
//! - Runtime lifecycle management (ActrNode lifecycle for Executor execution bodies)
//!
//! ### Runtime Infrastructure (formerly actr-runtime)
//!
//! - **Actor Lifecycle**: system init, node start/stop (ActrNode / ActrRef)
//! - **Message Transport**: layered architecture (Wire -> Transport -> Gate -> Dispatch)
//! - **Communication Modes**: in-process (zero-copy) and cross-process (WebRTC / WebSocket)
//! - **Message Persistence**: SQLite-backed Mailbox (ACID guarantees)
//! - **Observability**: logging, distributed tracing (OpenTelemetry, optional feature)
//! - **WASM Engine**: WASM actor execution (optional feature)
//!
//! ## Architecture Layers
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  Platform (Hyper)                                   │  AIS Bootstrap
//! │  Sandbox / Verify / Storage / KeyCache              │  Package Verify
//! ├─────────────────────────────────────────────────────┤
//! │  Lifecycle Management (ActrNode → ActrRef)
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 3: Inbound Dispatch                          │  DataStreamRegistry
//! │           (Fast Path Routing)                       │  MediaFrameRegistry
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 2: Outbound Gate                             │  HostGate
//! │           (Message Sending)                         │  PeerGate
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 1: Transport                                 │  Lane (core abstraction)
//! │           (Channel Management)                      │  HostTransport
//! │                                                     │  PeerTransport
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 0: Wire                                      │  WebRtcGate
//! │           (Physical Connections)                     │  WebRtcCoordinator
//! │                                                     │  SignalingClient
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Non-Goals
//!
//! Hyper does not understand business logic, does not perform business-level message routing,
//! and is unaware of business relationships between Actors.
//! The `hyper_send`/`hyper_recv` provided in WASM mode are network I/O primitives;
//! routing decisions are made by the ActrNode running inside the WASM.

// ═══════════════════════════════════════════════════════════════════════════════
// Platform modules (cross-platform)
// ═══════════════════════════════════════════════════════════════════════════════

pub mod config;
pub mod error;

// Runtime error re-exports (from actr_protocol, distinct from HyperError)
pub mod runtime_error;

// Verify module: PackageManifest struct is cross-platform,
// verification logic is native-only (sha2, ed25519-dalek).
pub mod verify;

// ═══════════════════════════════════════════════════════════════════════════════
// Native-only modules (excluded on wasm32)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_arch = "wasm32"))]
pub mod actr_ref;
#[cfg(not(target_arch = "wasm32"))]
pub mod ais_client;
#[cfg(not(target_arch = "wasm32"))]
pub mod key_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;
#[cfg(not(target_arch = "wasm32"))]
pub mod storage;

// Runtime infrastructure modules (native-only)
#[cfg(not(target_arch = "wasm32"))]
pub mod inbound;
#[cfg(not(target_arch = "wasm32"))]
pub mod lifecycle;
#[cfg(not(target_arch = "wasm32"))]
pub mod outbound;
#[cfg(not(target_arch = "wasm32"))]
pub mod transport;
#[cfg(not(target_arch = "wasm32"))]
pub mod wire;

// Shared helpers for integration tests (native-only)
#[cfg(all(not(target_arch = "wasm32"), feature = "test-utils"))]
pub mod test_support;

// Context and context factory (native-only, depend on transport/wire)
#[cfg(not(target_arch = "wasm32"))]
pub mod context;
#[cfg(not(target_arch = "wasm32"))]
pub mod context_factory;

// Runtime workload abstraction (native-only, WASM/dynclib host)
#[cfg(not(target_arch = "wasm32"))]
pub mod workload;

// WASM actor execution engine (optional, native-only)
#[cfg(all(not(target_arch = "wasm32"), feature = "wasm-engine"))]
pub mod wasm;

// Dynclib actor execution engine (optional, native-only)
#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
pub mod dynclib;

// Monitoring, observability, and resource management (native-only)
#[cfg(not(target_arch = "wasm32"))]
pub mod monitoring;
#[cfg(not(target_arch = "wasm32"))]
pub mod observability;
#[cfg(not(target_arch = "wasm32"))]
pub mod resource;

// ═══════════════════════════════════════════════════════════════════════════════
// Re-exports: Cross-platform
// ═══════════════════════════════════════════════════════════════════════════════

pub use config::{HyperConfig, TrustMode};
pub use error::{HyperError, HyperResult};
pub use verify::PackageManifest;

// Core protocol types
pub use actr_protocol::{Acl, ActrId, ActrType, ServiceSpec};

// Re-export MediaSample and MediaType from framework (dependency inversion)
pub use actr_framework::{MediaSample, MediaType};

// Runtime error types (distinct from HyperError — these are actor-facing errors)
pub use runtime_error::{ActorResult, ActrError, Classify, ErrorKind};

// Platform traits re-exports
pub use actr_platform_traits::{CryptoProvider, KvStore, PlatformError, PlatformProvider};

// ═══════════════════════════════════════════════════════════════════════════════
// Re-exports: Native-only
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_arch = "wasm32"))]
pub use ais_client::AisClient;
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::{ActorRuntime, ActrSystemHandle, WasmInstanceHandle};
#[cfg(not(target_arch = "wasm32"))]
pub use storage::ActorStore;
#[cfg(not(target_arch = "wasm32"))]
pub use verify::MfrCertCache;

// Observability
#[cfg(not(target_arch = "wasm32"))]
pub use observability::{ObservabilityGuard, init_observability};

#[cfg(not(target_arch = "wasm32"))]
pub use actr_ref::ActrRef;
// Runtime core structures
#[cfg(not(target_arch = "wasm32"))]
pub use lifecycle::{ActrNode, CredentialState, NetworkEventHandle};

// Layer 3: Inbound dispatch layer
#[cfg(not(target_arch = "wasm32"))]
pub use inbound::{DataStreamCallback, DataStreamRegistry, MediaFrameRegistry, MediaTrackCallback};

// Layer 2: Outbound gate abstraction layer
#[cfg(not(target_arch = "wasm32"))]
pub use outbound::{Gate, HostGate, PeerGate};

// Layer 1: Transport layer
#[cfg(not(target_arch = "wasm32"))]
pub use transport::{
    DataLane, DefaultWireBuilder, DefaultWireBuilderConfig, Dest, DestTransport,
    ExponentialBackoff, HostTransport, NetworkError, NetworkResult, PeerTransport, WireBuilder,
    WireHandle,
};

// Layer 0: Wire layer
#[cfg(not(target_arch = "wasm32"))]
pub use wire::{
    AuthConfig, AuthType, IceServer, ReconnectConfig, SignalingClient, SignalingConfig,
    SignalingEvent, SignalingStats, WebRtcConfig, WebRtcCoordinator, WebRtcGate, WebRtcNegotiator,
    WebSocketConnection, WebSocketGate, WebSocketServer, WebSocketSignalingClient, WsAuthContext,
};

// Mailbox (from actr-runtime-mailbox crate)
#[cfg(not(target_arch = "wasm32"))]
pub use actr_runtime_mailbox::{
    Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus,
};

// Context factory
#[cfg(not(target_arch = "wasm32"))]
pub use context_factory::ContextFactory;

// Monitoring and resource management
#[cfg(not(target_arch = "wasm32"))]
pub use monitoring::{Alert, AlertConfig, AlertSeverity, Monitor, MonitoringConfig};
#[cfg(not(target_arch = "wasm32"))]
pub use resource::{ResourceConfig, ResourceManager, ResourceQuota, ResourceUsage};

// Runtime workload abstraction
#[cfg(not(target_arch = "wasm32"))]
pub use workload::{
    HostAbiFn, HostOperation, HostOperationResult, InvocationContext, Workload,
    WorkloadDispatchResult,
};

// AIS key cache
#[cfg(not(target_arch = "wasm32"))]
pub use key_cache::AisKeyCache;

// ═══════════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════════

pub const INITIAL_CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

// ═══════════════════════════════════════════════════════════════════════════════
// Prelude
// ═══════════════════════════════════════════════════════════════════════════════

pub mod prelude {
    //! Convenience prelude module
    //!
    //! Re-exports commonly used types and traits for quick imports:
    //!
    //! ```rust
    //! use actr_hyper::prelude::*;
    //! ```

    // ── Platform types (cross-platform) ─────────────────────────────────────
    pub use crate::verify::PackageManifest;
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::{Hyper, storage::ActorStore};
    pub use crate::{HyperConfig, HyperError, HyperResult, TrustMode};

    // ── Core structures (native-only) ───────────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::actr_ref::ActrRef;
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::lifecycle::{ActrNode, CompatLockFile, CompatLockManager, CompatibilityCheck};

    // ── Layer 3: Inbound dispatch (native-only) ─────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::inbound::{
        DataStreamCallback, DataStreamRegistry, MediaFrameRegistry, MediaTrackCallback,
    };

    // Re-export MediaSample and MediaType from framework (dependency inversion)
    pub use actr_framework::{MediaSample, MediaType};

    // ── Layer 2: Outbound gate (native-only) ────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::outbound::{Gate, HostGate, PeerGate};

    // ── Context (native-only) ───────────────────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::context_factory::ContextFactory;

    // ── Layer 0: Wire / WebRTC (native-only) ────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::wire::webrtc::{
        AuthConfig, AuthType, IceServer, ReconnectConfig, SignalingClient, SignalingConfig,
        WebRtcConfig, WebRtcCoordinator, WebRtcGate, WebRtcNegotiator, WebSocketSignalingClient,
    };

    // ── Mailbox (native-only) ───────────────────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use actr_runtime_mailbox::{
        Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus,
    };

    // ── Layer 1: Transport (native-only) ────────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::transport::{
        DataLane, DefaultWireBuilder, DefaultWireBuilderConfig, Dest, DestTransport, HostTransport,
        NetworkError, NetworkResult, PeerTransport, WireBuilder, WireHandle,
    };

    // ── Error types ─────────────────────────────────────────────────────────
    pub use crate::runtime_error::{ActorResult, ActrError};

    // ── Monitoring / Resource (native-only) ─────────────────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::monitoring::{Alert, AlertSeverity, Monitor};
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::resource::{ResourceManager, ResourceQuota, ResourceUsage};

    // ── Base types ──────────────────────────────────────────────────────────
    pub use actr_protocol::ActrId;

    // ── Framework traits (for implementing Workload) ────────────────────────
    pub use actr_framework::{Context, Workload};

    // ── Async trait support ─────────────────────────────────────────────────
    pub use async_trait::async_trait;

    // ── Common utilities ────────────────────────────────────────────────────
    pub use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
    pub use chrono::{DateTime, Utc};
    pub use uuid::Uuid;

    // ── Tokio runtime primitives ────────────────────────────────────────────
    pub use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
    #[cfg(not(target_arch = "wasm32"))]
    pub use tokio::time::{Duration, Instant, sleep, timeout};

    // ── Logging ─────────────────────────────────────────────────────────────
    pub use tracing::{debug, error, info, trace, warn};
}

// ═══════════════════════════════════════════════════════════════════════════════
// Hyper runtime instance (platform singleton) — native-only
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
use std::io::Write;
#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(all(not(target_arch = "wasm32"), test))]
use base64::Engine as _;
#[cfg(not(target_arch = "wasm32"))]
use prost::Message;
#[cfg(not(target_arch = "wasm32"))]
use tracing::{debug, error, info, warn};
#[cfg(not(target_arch = "wasm32"))]
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use actr_platform_traits::KvOp;
#[cfg(not(target_arch = "wasm32"))]
use actr_protocol::{Realm, RegisterRequest, register_response};

#[cfg(not(target_arch = "wasm32"))]
/// Hyper runtime instance
///
/// Process-level singleton, initialized via `Hyper::init()`.
/// Holds resolved configuration, instance_id, namespace resolver, and other process-level state.
#[derive(Clone)]
pub struct Hyper {
    inner: Arc<HyperInner>,
}

#[cfg(not(target_arch = "wasm32"))]
struct HyperInner {
    config: HyperConfig,
    /// Locally unique ID generated and persisted on first startup
    instance_id: String,
    /// Package signature verifier
    verifier: verify::PackageVerifier,
    /// Optional platform provider for cross-platform abstraction
    platform: Option<Arc<dyn PlatformProvider>>,
}

#[cfg(not(target_arch = "wasm32"))]
/// Execution backend selected from a verified `.actr` package target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageExecutionBackend {
    /// Execute the package binary with the WASM runtime.
    Wasm,
    /// Execute the package binary as a native shared library.
    Cdylib,
}

#[cfg(not(target_arch = "wasm32"))]
/// Public `.actr` package input object consumed by Hyper.
#[derive(Debug, Clone)]
pub struct WorkloadPackage {
    bytes: bytes::Bytes,
}

#[cfg(not(target_arch = "wasm32"))]
impl WorkloadPackage {
    pub fn new(bytes: impl Into<bytes::Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Result of verifying a package and preparing a runtime workload from it.
pub struct LoadedWorkload {
    /// Verified package manifest retained for downstream bootstrap and storage operations.
    pub manifest: PackageManifest,
    /// Backend selected from `manifest.binary_target`.
    pub backend: PackageExecutionBackend,
    /// Ready-to-attach runtime workload.
    pub workload: crate::workload::Workload,
}

#[cfg(not(target_arch = "wasm32"))]
impl LoadedWorkload {
    /// Consume the wrapper and return its individual components.
    pub fn into_parts(
        self,
    ) -> (
        PackageManifest,
        PackageExecutionBackend,
        crate::workload::Workload,
    ) {
        (self.manifest, self.backend, self.workload)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Debug for LoadedWorkload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedWorkload")
            .field("manifest", &self.manifest)
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Hyper {
    /// Initialize Hyper (process-level, call once)
    ///
    /// - Parse configuration
    /// - Load or generate instance_id (persisted to data_dir)
    /// - Initialize package verifier
    pub async fn init(config: HyperConfig) -> HyperResult<Self> {
        Self::init_inner(config, None).await
    }

    /// Initialize Hyper with a platform provider for cross-platform support.
    ///
    /// When a `PlatformProvider` is injected:
    /// - `ensure_dir` delegates to `platform.ensure_dir()` instead of `tokio::fs`
    /// - `load_or_create_instance_id` delegates to `platform.load_or_create_instance_id()`
    /// - `bootstrap_credential` uses `platform.open_kv_store()` instead of `ActorStore::open()`
    /// - `PackageVerifier` receives `platform.crypto()` for signature verification
    pub async fn init_with_platform(
        config: HyperConfig,
        platform: Arc<dyn PlatformProvider>,
    ) -> HyperResult<Self> {
        Self::init_inner(config, Some(platform)).await
    }

    async fn init_inner(
        config: HyperConfig,
        platform: Option<Arc<dyn PlatformProvider>>,
    ) -> HyperResult<Self> {
        info!(
            data_dir = %config.data_dir.display(),
            trust_mode = match &config.trust_mode {
                TrustMode::Production { .. } => "production",
                TrustMode::Development { .. } => "development",
            },
            "Hyper initializing"
        );

        // ensure data_dir exists
        let data_dir_str = config.data_dir.to_string_lossy().to_string();
        if let Some(ref p) = platform {
            p.ensure_dir(&data_dir_str).await.map_err(|e| {
                HyperError::Config(format!(
                    "failed to create data_dir `{}`: {e}",
                    config.data_dir.display()
                ))
            })?;
        } else {
            tokio::fs::create_dir_all(&config.data_dir)
                .await
                .map_err(|e| {
                    HyperError::Config(format!(
                        "failed to create data_dir `{}`: {e}",
                        config.data_dir.display()
                    ))
                })?;
        }

        // load or generate instance_id
        let instance_id = if let Some(ref p) = platform {
            p.load_or_create_instance_id(&data_dir_str)
                .await
                .map_err(|e| HyperError::Storage(format!("failed to load instance_id: {e}")))?
        } else {
            load_or_create_instance_id(&config.data_dir).await?
        };
        debug!(instance_id, "Hyper instance_id ready");

        let mut verifier = verify::PackageVerifier::new(config.trust_mode.clone());
        if let Some(ref p) = platform {
            verifier = verifier.with_crypto(p.crypto());
        }

        Ok(Self {
            inner: Arc::new(HyperInner {
                config,
                instance_id,
                verifier,
                platform,
            }),
        })
    }

    /// Verify a [`WorkloadPackage`] and return the verified manifest.
    ///
    /// Successful verification means:
    /// - binary_hash matches the recomputed result (package not tampered with)
    /// - MFR signature is valid (from a trusted manufacturer)
    ///
    /// In production mode, the MFR public key is fetched asynchronously first (requires AIS reachability);
    /// in development mode, there are no network calls and verification is fully synchronous.
    pub async fn verify_package(&self, package: &WorkloadPackage) -> HyperResult<PackageManifest> {
        let bytes = package.bytes();
        // production mode: prefetch MFR public key (write to cert_cache), then verify synchronously
        if matches!(&self.inner.config.trust_mode, TrustMode::Production { .. }) {
            if let Some((manufacturer, signing_key_id)) = quick_extract_manifest_info(bytes) {
                debug!(
                    manufacturer,
                    ?signing_key_id,
                    "production mode: prefetching MFR public key"
                );
                self.inner
                    .verifier
                    .prefetch_mfr_cert(&manufacturer, signing_key_id.as_deref())
                    .await?;
            }
        }
        self.inner.verifier.verify(bytes)
    }

    /// Verify a package, select the execution backend from `binary.target`,
    /// and prepare a runtime workload ready for node attachment.
    ///
    /// For WASM packages, Hyper initialises the guest with an empty credential payload so
    /// the caller can bootstrap AIS credentials afterwards and inject them before start.
    /// For dynclib packages, Hyper initialises the guest with an empty JSON object.
    pub async fn load_workload_package(
        &self,
        package: &WorkloadPackage,
    ) -> HyperResult<LoadedWorkload> {
        let bytes = package.bytes();
        let manifest = self.verify_package(package).await?;
        let backend = select_package_execution_backend(&manifest)?;
        let workload = match backend {
            PackageExecutionBackend::Wasm => self.load_wasm_workload(bytes, &manifest),
            PackageExecutionBackend::Cdylib => self.load_dynclib_workload(bytes, &manifest),
        }?;

        Ok(LoadedWorkload {
            manifest,
            backend,
            workload,
        })
    }

    /// Verify and load a [`WorkloadPackage`], then build a fully initialized [`ActrNode`].
    ///
    /// This is the primary entry point for package-driven actors. It replaces the manual
    /// sequence of package loading followed by node construction.
    pub async fn attach_package(
        &self,
        package: &WorkloadPackage,
        config: actr_config::Config,
    ) -> HyperResult<crate::lifecycle::ActrNode> {
        let loaded = self.load_workload_package(package).await?;
        let node =
            crate::lifecycle::ActrNode::build(config, loaded.workload, Some(loaded.manifest))
                .await
                .map_err(|e| HyperError::Runtime(e.to_string()))?;
        Ok(node)
    }

    /// Bootstrap credential registration with AIS using the package manifest stored in `node`.
    pub async fn bootstrap_node_credential(
        &self,
        node: &crate::lifecycle::ActrNode,
        ais_endpoint: &str,
        realm_id: u32,
        service_spec: Option<ServiceSpec>,
        acl: Option<Acl>,
    ) -> HyperResult<register_response::RegisterOk> {
        let manifest = node.package_manifest().ok_or_else(|| {
            HyperError::InvalidManifest(
                "node does not carry a verified package manifest".to_string(),
            )
        })?;
        self.bootstrap_credential(manifest, ais_endpoint, realm_id, service_spec, acl)
            .await
    }

    fn load_wasm_workload(
        &self,
        bytes: &[u8],
        manifest: &PackageManifest,
    ) -> HyperResult<crate::workload::Workload> {
        #[cfg(feature = "wasm-engine")]
        {
            let wasm_bytes = actr_pack::load_binary(bytes).map_err(|e| {
                HyperError::Runtime(format!(
                    "failed to extract package binary `{}` for target `{}`: {e}",
                    manifest.binary_path, manifest.binary_target
                ))
            })?;
            let host = crate::wasm::WasmHost::compile(&wasm_bytes).map_err(|e| {
                HyperError::Runtime(format!(
                    "failed to compile WASM package target `{}`: {e}",
                    manifest.binary_target
                ))
            })?;
            let mut instance = host.instantiate().map_err(|e| {
                HyperError::Runtime(format!(
                    "failed to instantiate WASM package target `{}`: {e}",
                    manifest.binary_target
                ))
            })?;
            instance
                .init(&actr_framework::guest::abi::InitPayloadV1 {
                    version: actr_framework::guest::abi::version::V1,
                    actr_type: manifest.actr_type_str(),
                    credential: Vec::new(),
                    actor_id: Vec::new(),
                    realm_id: 0,
                })
                .map_err(|e| {
                    HyperError::Runtime(format!(
                        "failed to initialize WASM package target `{}`: {e}",
                        manifest.binary_target
                    ))
                })?;
            Ok(crate::workload::Workload::Wasm(instance))
        }

        #[cfg(not(feature = "wasm-engine"))]
        {
            let _ = (bytes, manifest);
            Err(HyperError::Runtime(
                "package target requires the `wasm-engine` feature, but it is not enabled"
                    .to_string(),
            ))
        }
    }

    fn load_dynclib_workload(
        &self,
        bytes: &[u8],
        manifest: &PackageManifest,
    ) -> HyperResult<crate::workload::Workload> {
        #[cfg(feature = "dynclib-engine")]
        {
            let cache_path =
                ensure_dynclib_cache_path(&self.inner.config.data_dir, bytes, manifest)?;
            let host = load_dynclib_host_with_rebuild(&cache_path, bytes, manifest)?;
            let instance = host
                .instantiate(&actr_framework::guest::abi::InitPayloadV1 {
                    version: actr_framework::guest::abi::version::V1,
                    actr_type: manifest.actr_type_str(),
                    credential: Vec::new(),
                    actor_id: Vec::new(),
                    realm_id: 0,
                })
                .map_err(|e| {
                    HyperError::Runtime(format!(
                        "failed to initialize dynclib package target `{}`: {e}",
                        manifest.binary_target
                    ))
                })?;

            Ok(crate::workload::Workload::DynClib(
                crate::dynclib::DynClibWorkload::new(host, instance),
            ))
        }

        #[cfg(not(feature = "dynclib-engine"))]
        {
            let _ = (bytes, manifest);
            Err(HyperError::Runtime(
                "package target requires the `dynclib-engine` feature, but it is not enabled"
                    .to_string(),
            ))
        }
    }

    /// Resolve the storage namespace path for a verified manifest
    ///
    /// The path is fixed here; all subsequent storage operations are isolated based on this path.
    pub fn resolve_storage_path(&self, manifest: &PackageManifest) -> HyperResult<PathBuf> {
        let resolver = config::NamespaceResolver::new(&self.inner.config, &self.inner.instance_id)?
            .with_actor_type(
                &manifest.manufacturer,
                &manifest.actr_name,
                &manifest.version,
            );
        resolver.resolve(&self.inner.config.storage_path_template)
    }

    /// Bootstrap credential registration with AIS (two-phase flow)
    ///
    /// Hyper completes registration bootstrap on behalf of the Actor and returns the full AIS
    /// registration payload.
    ///
    /// ## Two-Phase Logic
    ///
    /// - **Phase 1 (first registration)**: no valid PSK in ActorStore ->
    ///   register with MFR-signed manifest -> AIS returns credential + PSK -> stored in ActorStore
    /// - **Phase 2 (PSK renewal)**: valid PSK exists in ActorStore ->
    ///   register directly with PSK -> AIS returns new credential
    ///
    /// ## Parameters
    ///
    /// - `manifest`: verified package manifest (from `verify_package`)
    /// - `ais_endpoint`: AIS HTTP address, e.g. `"http://ais.example.com:8080"`
    /// - `realm_id`: target Realm ID
    /// - `service_spec`: optional protobuf API metadata published to discovery
    /// - `acl`: optional access-control policy attached to the actor
    pub async fn bootstrap_credential(
        &self,
        manifest: &PackageManifest,
        ais_endpoint: &str,
        realm_id: u32,
        service_spec: Option<ServiceSpec>,
        acl: Option<Acl>,
    ) -> HyperResult<register_response::RegisterOk> {
        info!(
            actr_type = manifest.actr_type_str(),
            ais_endpoint, realm_id, "starting credential bootstrap with AIS"
        );

        // 1. Open the Actor's storage (platform-agnostic KV store or ActorStore)
        let storage_path = self.resolve_storage_path(manifest)?;
        let store: Arc<dyn KvStore> = if let Some(ref platform) = self.inner.platform {
            let ns = storage_path.to_string_lossy().to_string();
            platform
                .open_kv_store(&ns)
                .await
                .map_err(|e| HyperError::Storage(format!("failed to open KV store: {e}")))?
        } else {
            Arc::new(ActorStore::open(&storage_path).await?)
        };

        // 2. Check if there is a valid PSK in ActorStore
        let valid_psk = load_valid_psk_dyn(&*store).await?;

        // 3. Build RegisterRequest and send to AIS
        let ais = AisClient::new(ais_endpoint);

        let actr_type = ActrType {
            manufacturer: manifest.manufacturer.clone(),
            name: manifest.actr_name.clone(),
            version: manifest.version.clone(),
        };
        let realm = Realm { realm_id };

        let response = if let Some(psk_token) = valid_psk {
            // Phase 2: PSK renewal
            debug!(
                actr_type = manifest.actr_type_str(),
                "renewing credential using PSK"
            );
            let req = RegisterRequest {
                actr_type,
                realm,
                service_spec,
                acl,
                service: None,
                ws_address: None,
                manifest_raw: None,
                mfr_signature: None,
                psk_token: Some(psk_token.into()),
                target: Some(manifest.target.clone()),
            };
            ais.register_with_psk(req).await?
        } else {
            // Phase 1: first registration, carrying MFR manifest
            info!(
                actr_type = manifest.actr_type_str(),
                "first registration: registering with AIS using MFR manifest"
            );

            let req = RegisterRequest {
                actr_type,
                realm,
                service_spec,
                acl,
                service: None,
                ws_address: None,
                manifest_raw: Some(manifest.manifest_raw.clone().into()),
                mfr_signature: Some(manifest.signature.clone().into()),
                psk_token: None,
                target: Some(manifest.target.clone()),
            };
            ais.register_with_manifest(req).await?
        };

        // 4. Process AIS response
        let ok = match response.result {
            Some(register_response::Result::Success(ok)) => ok,
            Some(register_response::Result::Error(e)) => {
                error!(
                    actr_type = manifest.actr_type_str(),
                    error_code = e.code,
                    error_message = %e.message,
                    "AIS registration returned error"
                );
                return Err(HyperError::AisBootstrapFailed(format!(
                    "AIS rejected registration (code={}): {}",
                    e.code, e.message
                )));
            }
            None => {
                error!(
                    actr_type = manifest.actr_type_str(),
                    "AIS response missing result field"
                );
                return Err(HyperError::AisBootstrapFailed(
                    "AIS response missing result field".to_string(),
                ));
            }
        };

        // 5a. If the response contains a PSK (first registration scenario), store it in ActorStore
        if let (Some(psk), Some(psk_expires_at)) = (&ok.psk, ok.psk_expires_at) {
            info!(
                actr_type = manifest.actr_type_str(),
                psk_expires_at, "received PSK from AIS, storing in ActorStore"
            );
            let expires_at_bytes = (psk_expires_at as u64).to_le_bytes().to_vec();
            store
                .batch(vec![
                    KvOp::Set {
                        key: "hyper:psk:token".to_string(),
                        value: psk.to_vec(),
                    },
                    KvOp::Set {
                        key: "hyper:psk:expires_at".to_string(),
                        value: expires_at_bytes,
                    },
                ])
                .await
                .map_err(|e| HyperError::Storage(format!("failed to store PSK: {e}")))?;
            debug!(
                actr_type = manifest.actr_type_str(),
                "PSK successfully persisted to ActorStore"
            );
        }

        // 5b. Store signing_pubkey + signing_key_id (for AisKeyCache use)
        let pubkey_bytes = ok.signing_pubkey.to_vec();
        let key_id_bytes = ok.signing_key_id.to_le_bytes().to_vec();
        store
            .batch(vec![
                KvOp::Set {
                    key: "hyper:ais:signing_pubkey".to_string(),
                    value: pubkey_bytes,
                },
                KvOp::Set {
                    key: "hyper:ais:signing_key_id".to_string(),
                    value: key_id_bytes,
                },
            ])
            .await
            .map_err(|e| HyperError::Storage(format!("failed to store signing key: {e}")))?;
        debug!(
            actr_type = manifest.actr_type_str(),
            signing_key_id = ok.signing_key_id,
            "AIS signing public key persisted to ActorStore"
        );

        info!(
            actr_type = manifest.actr_type_str(),
            credential_len = ok.credential.encode_to_vec().len(),
            "AIS credential bootstrap succeeded"
        );

        Ok(ok)
    }

    /// Current instance_id
    pub fn instance_id(&self) -> &str {
        &self.inner.instance_id
    }

    /// Current configuration
    pub fn config(&self) -> &HyperConfig {
        &self.inner.config
    }
}

// ─── Helper functions (native-only) ──────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
/// Load PSK from any KvStore implementation; returns PSK bytes if present and not expired
///
/// PSK expiration check: considered expired when current Unix timestamp (seconds) >= expires_at.
async fn load_valid_psk_dyn(store: &dyn KvStore) -> HyperResult<Option<Vec<u8>>> {
    let token = store
        .get("hyper:psk:token")
        .await
        .map_err(|e| HyperError::Storage(format!("failed to read PSK token: {e}")))?;
    let expires_at_raw = store
        .get("hyper:psk:expires_at")
        .await
        .map_err(|e| HyperError::Storage(format!("failed to read PSK expires_at: {e}")))?;

    check_psk_expiry(token, expires_at_raw)
}

/// Load PSK from ActorStore; returns PSK bytes if present and not expired, otherwise None
///
/// PSK expiration check: considered expired when current Unix timestamp (seconds) >= expires_at.
#[cfg(all(not(target_arch = "wasm32"), test))]
async fn load_valid_psk(store: &ActorStore) -> HyperResult<Option<Vec<u8>>> {
    let token = store.kv_get("hyper:psk:token").await?;
    let expires_at_raw = store.kv_get("hyper:psk:expires_at").await?;

    check_psk_expiry(token, expires_at_raw)
}

#[cfg(not(target_arch = "wasm32"))]
/// Check PSK expiry given pre-fetched token and expires_at values
fn check_psk_expiry(
    token: Option<Vec<u8>>,
    expires_at_raw: Option<Vec<u8>>,
) -> HyperResult<Option<Vec<u8>>> {
    match (token, expires_at_raw) {
        (Some(token), Some(expires_bytes)) => {
            // parse expiration time (u64 little-endian)
            if expires_bytes.len() != 8 {
                warn!("PSK expires_at has unexpected format, falling back to first registration");
                return Ok(None);
            }
            let expires_at = u64::from_le_bytes(expires_bytes.as_slice().try_into().unwrap());

            // get current Unix timestamp (seconds)
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if now_secs >= expires_at {
                warn!(
                    psk_expires_at = expires_at,
                    now = now_secs,
                    "PSK expired, falling back to first registration"
                );
                Ok(None)
            } else {
                debug!(
                    psk_expires_at = expires_at,
                    now = now_secs,
                    remaining_secs = expires_at - now_secs,
                    "PSK valid, using PSK renewal path"
                );
                Ok(Some(token))
            }
        }
        _ => {
            debug!("no PSK in ActorStore, proceeding with first registration");
            Ok(None)
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), test))]
/// Serialize a verified package manifest into the JSON payload expected by tests
/// and downstream manifest forwarding helpers.
fn build_manifest_json(manifest: &PackageManifest) -> HyperResult<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "manufacturer": manifest.manufacturer,
        "actr_name": manifest.actr_name,
        "version": manifest.version,
        "binary_path": manifest.binary_path,
        "binary_target": manifest.binary_target,
        "binary_hash": hex::encode(manifest.binary_hash),
        "capabilities": manifest.capabilities,
        "signature": base64::engine::general_purpose::STANDARD.encode(&manifest.signature),
        "target": manifest.target,
    }))
    .map_err(|e| HyperError::Runtime(format!("failed to serialize manifest JSON: {e}")))
}

#[cfg(not(target_arch = "wasm32"))]
/// Quickly extract the `manufacturer` and `signing_key_id` fields from an `.actr` package,
/// used only to prefetch the MFR public key without full verification.
///
/// Returns `None` when parsing fails or the format is not recognized.
/// The caller then skips prefetch and lets verification report the error.
fn quick_extract_manifest_info(bytes: &[u8]) -> Option<(String, Option<String>)> {
    if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
        return actr_pack::read_manifest(bytes)
            .ok()
            .map(|m| (m.manufacturer, m.signing_key_id));
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn select_package_execution_backend(
    manifest: &PackageManifest,
) -> HyperResult<PackageExecutionBackend> {
    if manifest.is_wasm_target() {
        return Ok(PackageExecutionBackend::Wasm);
    }

    if is_compatible_native_target(&manifest.binary_target) {
        return Ok(PackageExecutionBackend::Cdylib);
    }

    Err(HyperError::InvalidManifest(format!(
        "unsupported binary target `{}` for host `{}-{}`; expected `wasm32-*` or a native target matching this host",
        manifest.binary_target,
        std::env::consts::ARCH,
        std::env::consts::OS,
    )))
}

/// Check that `target` is a valid Rust target triple compatible with the current host.
///
/// A target triple has at least 3 segments (arch-vendor-os or arch-vendor-os-env).
/// We verify that the arch and OS components match the running host to reject
/// cross-platform cdylib packages early, rather than failing at `dlopen` time.
#[cfg(not(target_arch = "wasm32"))]
fn is_compatible_native_target(target: &str) -> bool {
    let segments: Vec<&str> = target.split('-').filter(|s| !s.is_empty()).collect();
    if segments.len() < 3 {
        return false;
    }

    let target_arch = segments[0];
    // OS is typically the third segment (arch-vendor-os[-env]).
    let target_os = segments[2];

    // Normalize arch names: Rust target triples use different names than std::env::consts::ARCH.
    let arch_matches = match (target_arch, std::env::consts::ARCH) {
        (a, b) if a == b => true,
        ("x86_64", "x86_64") => true,
        ("aarch64", "aarch64") => true,
        _ => false,
    };

    // Normalize OS names: Rust target triples use e.g. "darwin" while consts::OS is "macos".
    let os_matches = match (target_os, std::env::consts::OS) {
        (a, b) if a == b => true,
        ("darwin", "macos") | ("macos", "darwin") => true,
        _ => false,
    };

    arch_matches && os_matches
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "dynclib-engine",
    target_os = "macos"
))]
fn dynclib_tempfile_suffix() -> &'static str {
    ".dylib"
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "dynclib-engine",
    target_os = "linux"
))]
fn dynclib_tempfile_suffix() -> &'static str {
    ".so"
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "dynclib-engine",
    target_os = "windows"
))]
fn dynclib_tempfile_suffix() -> &'static str {
    ".dll"
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "dynclib-engine",
    not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
))]
fn dynclib_tempfile_suffix() -> &'static str {
    ".dynlib"
}

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
const DYNCLIB_CACHE_DIR: &str = "dynclib-cache";

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
fn dynclib_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DYNCLIB_CACHE_DIR)
}

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
fn dynclib_cache_path(data_dir: &Path, binary_hash: &[u8; 32]) -> PathBuf {
    dynclib_cache_dir(data_dir).join(format!(
        "{}{}",
        hex::encode(binary_hash),
        dynclib_tempfile_suffix()
    ))
}

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
fn extract_dynclib_binary(bytes: &[u8], manifest: &PackageManifest) -> HyperResult<Vec<u8>> {
    actr_pack::load_binary(bytes).map_err(|e| {
        HyperError::Runtime(format!(
            "failed to extract package binary `{}` for target `{}`: {e}",
            manifest.binary_path, manifest.binary_target
        ))
    })
}

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
fn write_dynclib_cache_file(cache_path: &Path, binary_bytes: &[u8]) -> HyperResult<()> {
    let cache_dir = cache_path.parent().ok_or_else(|| {
        HyperError::Runtime("dynclib cache path has no parent directory".to_string())
    })?;
    std::fs::create_dir_all(cache_dir).map_err(|e| {
        HyperError::Runtime(format!(
            "failed to create dynclib cache directory `{}`: {e}",
            cache_dir.display()
        ))
    })?;

    let mut temp_file = tempfile::Builder::new()
        .prefix("actr-dynclib-")
        .tempfile_in(cache_dir)
        .map_err(|e| {
            HyperError::Runtime(format!(
                "failed to allocate dynclib cache temp file in `{}`: {e}",
                cache_dir.display()
            ))
        })?;

    temp_file.write_all(binary_bytes).map_err(|e| {
        HyperError::Runtime(format!(
            "failed to write dynclib cache temp file `{}`: {e}",
            temp_file.path().display()
        ))
    })?;
    temp_file.flush().map_err(|e| {
        HyperError::Runtime(format!(
            "failed to flush dynclib cache temp file `{}`: {e}",
            temp_file.path().display()
        ))
    })?;

    match temp_file.persist_noclobber(cache_path) {
        Ok(_) => Ok(()),
        Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(HyperError::Runtime(format!(
            "failed to persist dynclib cache file `{}`: {}",
            cache_path.display(),
            err.error
        ))),
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
fn ensure_dynclib_cache_path(
    data_dir: &Path,
    bytes: &[u8],
    manifest: &PackageManifest,
) -> HyperResult<PathBuf> {
    let cache_path = dynclib_cache_path(data_dir, &manifest.binary_hash);
    if cache_path.exists() {
        return Ok(cache_path);
    }

    let binary_bytes = extract_dynclib_binary(bytes, manifest)?;
    write_dynclib_cache_file(&cache_path, &binary_bytes)?;
    Ok(cache_path)
}

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
fn rebuild_dynclib_cache_file(
    cache_path: &Path,
    bytes: &[u8],
    manifest: &PackageManifest,
) -> HyperResult<()> {
    match std::fs::remove_file(cache_path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(HyperError::Runtime(format!(
                "failed to remove corrupt dynclib cache file `{}`: {err}",
                cache_path.display()
            )));
        }
    }

    let binary_bytes = extract_dynclib_binary(bytes, manifest)?;
    write_dynclib_cache_file(cache_path, &binary_bytes)
}

#[cfg(all(not(target_arch = "wasm32"), feature = "dynclib-engine"))]
fn load_dynclib_host_with_rebuild(
    cache_path: &Path,
    bytes: &[u8],
    manifest: &PackageManifest,
) -> HyperResult<crate::dynclib::DynclibHost> {
    match crate::dynclib::DynclibHost::load(cache_path) {
        Ok(host) => Ok(host),
        Err(first_err) => {
            warn!(
                path = %cache_path.display(),
                target = %manifest.binary_target,
                error = %first_err,
                "cached dynclib load failed, rebuilding cache once"
            );
            rebuild_dynclib_cache_file(cache_path, bytes, manifest)?;
            crate::dynclib::DynclibHost::load(cache_path).map_err(|second_err| {
                HyperError::Runtime(format!(
                    "failed to load dynclib package target `{}` from cache `{}` after rebuild; first load error: {first_err}; second load error: {second_err}",
                    manifest.binary_target,
                    cache_path.display()
                ))
            })
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Load an existing `instance_id` or generate and persist a new one.
async fn load_or_create_instance_id(data_dir: &std::path::Path) -> HyperResult<String> {
    let id_file = data_dir.join(".hyper-instance-id");

    if id_file.exists() {
        let id = tokio::fs::read_to_string(&id_file)
            .await
            .map_err(|e| HyperError::Storage(format!("failed to read instance_id file: {e}")))?;
        let id = id.trim().to_string();
        if !id.is_empty() {
            return Ok(id);
        }
        warn!("instance_id file is empty; generating a new one");
    }

    let new_id = Uuid::new_v4().to_string();
    tokio::fs::write(&id_file, &new_id)
        .await
        .map_err(|e| HyperError::Storage(format!("failed to write instance_id file: {e}")))?;
    info!(instance_id = %new_id, "generated a new Hyper instance_id");
    Ok(new_id)
}

#[cfg(all(not(target_arch = "wasm32"), test))]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    #[cfg(feature = "dynclib-engine")]
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    fn dev_config(dir: &TempDir) -> HyperConfig {
        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey = signing_key.verifying_key().to_bytes().to_vec();
        HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Development {
            self_signed_pubkey: pubkey,
        })
    }

    #[tokio::test]
    async fn init_creates_data_dir_and_instance_id() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("subdir/nested");
        let config = dev_config(&TempDir::new().unwrap());
        let config = HyperConfig::new(&sub).with_trust_mode(config.trust_mode);

        let hyper = Hyper::init(config).await.unwrap();
        assert!(sub.exists());
        assert!(!hyper.instance_id().is_empty());
    }

    #[tokio::test]
    async fn instance_id_is_stable_across_reinit() {
        let dir = TempDir::new().unwrap();
        let config1 = dev_config(&dir);
        let hyper1 = Hyper::init(config1).await.unwrap();
        let id1 = hyper1.instance_id().to_string();

        let config2 = dev_config(&dir);
        let hyper2 = Hyper::init(config2).await.unwrap();
        let id2 = hyper2.instance_id().to_string();

        assert_eq!(id1, id2, "instance_id should remain stable across restarts");
    }

    #[tokio::test]
    async fn verify_package_rejects_non_wasm() {
        let dir = TempDir::new().unwrap();
        let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
        let result = hyper
            .verify_package(&WorkloadPackage::new(b"not a wasm file".to_vec()))
            .await;
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    #[tokio::test]
    async fn verify_package_rejects_non_actr_format() {
        let dir = TempDir::new().unwrap();
        let hyper = Hyper::init(dev_config(&dir)).await.unwrap();

        // Non-.actr bytes should return InvalidManifest
        let result = hyper
            .verify_package(&WorkloadPackage::new(b"\0asm\x01\x00\x00\x00".to_vec()))
            .await;
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    // ─── PSK storage and expiration unit tests ──────────────────────────────

    async fn open_test_store(dir: &TempDir) -> ActorStore {
        let db_path = dir.path().join("test.db");
        ActorStore::open(&db_path).await.unwrap()
    }

    /// Store a valid PSK and verify that load_valid_psk returns it.
    #[tokio::test]
    async fn psk_valid_returns_token() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let psk_token = b"test-psk-secret".to_vec();
        // Set the expiry time to one hour from now.
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;

        store.kv_set("hyper:psk:token", &psk_token).await.unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expires_at.to_le_bytes())
            .await
            .unwrap();

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, Some(psk_token), "A valid PSK should be returned");
    }

    /// Store an expired PSK and verify that load_valid_psk returns None.
    #[tokio::test]
    async fn psk_expired_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let psk_token = b"expired-psk".to_vec();
        // Set the expiry time to one second in the past.
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(1);

        store.kv_set("hyper:psk:token", &psk_token).await.unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expires_at.to_le_bytes())
            .await
            .unwrap();

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, None, "An expired PSK should return None");
    }

    /// load_valid_psk returns None when ActorStore has no PSK.
    #[tokio::test]
    async fn psk_absent_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, None, "Missing PSK should return None");
    }

    /// load_valid_psk returns None if token exists without expires_at.
    #[tokio::test]
    async fn psk_missing_expires_at_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store
            .kv_set("hyper:psk:token", b"orphan-token")
            .await
            .unwrap();
        // Intentionally leave expires_at unset.

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, None, "Missing expires_at should return None");
    }

    // ─── AIS integration tests (mockito mock server) ────────────────────────

    /// Helper: build a PackageManifest for tests.
    fn fake_manifest() -> PackageManifest {
        PackageManifest {
            manufacturer: "test-mfr".to_string(),
            actr_name: "TestActor".to_string(),
            version: "0.1.0".to_string(),
            binary_path: "bin/actor.wasm".to_string(),
            binary_target: "wasm32-wasip1".to_string(),
            binary_hash: [0u8; 32],
            capabilities: vec![],
            signature: vec![0u8; 64],
            manifest_raw: vec![],
            target: "wasm32-wasip1".to_string(),
        }
    }

    /// Helper: build valid RegisterResponse protobuf bytes with credential data.
    fn fake_register_response_bytes(with_psk: bool) -> Vec<u8> {
        use actr_protocol::{
            AIdCredential, ActrId, ActrType, IdentityClaims, Realm, RegisterResponse,
            TurnCredential, register_response,
        };

        let claims = IdentityClaims {
            realm_id: 1,
            actor_id: "test-actor-id".to_string(),
            expires_at: u64::MAX,
        };
        let claims_bytes = claims.encode_to_vec();

        let credential = AIdCredential {
            key_id: 1,
            claims: claims_bytes.into(),
            signature: vec![0u8; 64].into(),
        };

        let actr_id = ActrId {
            realm: Realm { realm_id: 1 },
            serial_number: 42,
            r#type: ActrType {
                manufacturer: "test-mfr".to_string(),
                name: "TestActor".to_string(),
                version: "0.1.0".to_string(),
            },
        };

        let turn = TurnCredential {
            username: "user".to_string(),
            password: "pass".to_string(),
            expires_at: u64::MAX,
        };

        let mut ok = register_response::RegisterOk {
            actr_id,
            credential,
            turn_credential: turn,
            credential_expires_at: None,
            signaling_heartbeat_interval_secs: 30,
            signing_pubkey: vec![0u8; 32].into(),
            signing_key_id: 1,
            psk: None,
            psk_expires_at: None,
        };

        if with_psk {
            ok.psk = Some(b"fresh-psk-from-ais".to_vec().into());
            ok.psk_expires_at = Some(
                (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 86400) as i64,
            );
        }

        RegisterResponse {
            result: Some(register_response::Result::Success(ok)),
        }
        .encode_to_vec()
    }

    fn test_service_spec() -> Option<ServiceSpec> {
        Some(ServiceSpec {
            name: "EchoService".to_string(),
            description: Some("test service".to_string()),
            fingerprint: "fp-123".to_string(),
            protobufs: vec![],
            published_at: None,
            tags: vec!["latest".to_string()],
        })
    }

    fn test_acl() -> Option<Acl> {
        Some(Acl { rules: vec![] })
    }

    #[test]
    fn compatible_native_target_matches_current_host() {
        // Current host should always match itself.
        let current = format!(
            "{}-unknown-{}",
            std::env::consts::ARCH,
            if std::env::consts::OS == "macos" {
                "darwin"
            } else {
                std::env::consts::OS
            }
        );
        assert!(
            is_compatible_native_target(&current),
            "current host target `{current}` should be compatible"
        );
    }

    #[test]
    fn compatible_native_target_rejects_cross_platform() {
        // A target for a different arch/os should be rejected.
        assert!(!is_compatible_native_target("riscv64gc-unknown-linux-gnu"));
        assert!(!is_compatible_native_target("s390x-unknown-linux-gnu"));
    }

    #[test]
    fn compatible_native_target_rejects_short_triples() {
        assert!(!is_compatible_native_target("invalid-target"));
        assert!(!is_compatible_native_target("single"));
        assert!(!is_compatible_native_target(""));
    }

    #[cfg(feature = "dynclib-engine")]
    fn fake_dynclib_manifest(binary_hash: [u8; 32]) -> PackageManifest {
        PackageManifest {
            manufacturer: "test-mfr".to_string(),
            actr_name: "DynActor".to_string(),
            version: "1.0.0".to_string(),
            binary_path: format!("bin/actor{}", dynclib_tempfile_suffix()),
            binary_target: format!(
                "{}-unknown-{}",
                std::env::consts::ARCH,
                if std::env::consts::OS == "macos" {
                    "darwin"
                } else {
                    std::env::consts::OS
                }
            ),
            binary_hash,
            capabilities: vec![],
            signature: vec![0u8; 64],
            manifest_raw: vec![],
            target: format!(
                "{}-unknown-{}",
                std::env::consts::ARCH,
                if std::env::consts::OS == "macos" {
                    "darwin"
                } else {
                    std::env::consts::OS
                }
            ),
        }
    }

    #[cfg(feature = "dynclib-engine")]
    #[test]
    fn dynclib_cache_path_uses_hash_and_platform_suffix() {
        let dir = TempDir::new().unwrap();
        let path = dynclib_cache_path(dir.path(), &[0xAB; 32]);

        assert_eq!(path.parent().unwrap(), dynclib_cache_dir(dir.path()));
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!("{}{}", hex::encode([0xAB; 32]), dynclib_tempfile_suffix())
        );
    }

    #[cfg(feature = "dynclib-engine")]
    #[test]
    fn ensure_dynclib_cache_path_preserves_existing_file() {
        let dir = TempDir::new().unwrap();
        let manifest = fake_dynclib_manifest([0x11; 32]);
        let initial_bytes = b"initial dylib bytes";
        let cache_path = ensure_dynclib_cache_path(dir.path(), initial_bytes, &manifest).unwrap();

        let replacement_bytes = b"replacement dylib bytes";
        let second_path =
            ensure_dynclib_cache_path(dir.path(), replacement_bytes, &manifest).unwrap();

        assert_eq!(cache_path, second_path);
        assert_eq!(std::fs::read(&cache_path).unwrap(), initial_bytes);
    }

    #[cfg(feature = "dynclib-engine")]
    #[test]
    fn ensure_dynclib_cache_path_handles_concurrent_creation() {
        let dir = TempDir::new().unwrap();
        let manifest = fake_dynclib_manifest([0x22; 32]);
        let bytes = Arc::new(b"shared dylib bytes".to_vec());
        let data_dir = Arc::new(dir.path().to_path_buf());
        let barrier = Arc::new(Barrier::new(3));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let data_dir = Arc::clone(&data_dir);
                let manifest = manifest.clone();
                let bytes = Arc::clone(&bytes);
                std::thread::spawn(move || {
                    barrier.wait();
                    ensure_dynclib_cache_path(&data_dir, &bytes, &manifest)
                })
            })
            .collect();

        barrier.wait();

        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect();

        assert_eq!(results[0], results[1]);
        assert_eq!(
            std::fs::read(&results[0]).unwrap(),
            bytes.as_ref().as_slice()
        );
    }

    /// First registration with no PSK should store the PSK returned by AIS.
    #[tokio::test]
    async fn bootstrap_first_registration_stores_psk() {
        let response_body = fake_register_response_bytes(true);

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(response_body)
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        let manifest = fake_manifest();
        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1, test_service_spec(), test_acl())
            .await;

        mock.assert_async().await;
        assert!(
            result.is_ok(),
            "Initial registration should succeed, got: {:?}",
            result.err()
        );

        // Verify the PSK was written to ActorStore.
        let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
        let store = ActorStore::open(&storage_path).await.unwrap();
        let psk = store.kv_get("hyper:psk:token").await.unwrap();
        assert!(
            psk.is_some(),
            "PSK should be stored in ActorStore after initial registration"
        );
        assert_eq!(psk.unwrap(), b"fresh-psk-from-ais".to_vec());
    }

    /// A valid PSK should skip manifest registration and use the renewal path.
    #[tokio::test]
    async fn bootstrap_psk_renewal_skips_manifest() {
        let response_body = fake_register_response_bytes(false);

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(response_body)
            .expect(1) // /register should be called exactly once.
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        // Seed ActorStore with a valid PSK.
        let manifest = fake_manifest();
        let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
        let store = ActorStore::open(&storage_path).await.unwrap();

        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        store
            .kv_set("hyper:psk:token", b"existing-valid-psk")
            .await
            .unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expires_at.to_le_bytes())
            .await
            .unwrap();

        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1, test_service_spec(), test_acl())
            .await;

        mock.assert_async().await;
        assert!(
            result.is_ok(),
            "PSK renewal should succeed, got: {:?}",
            result.err()
        );
    }

    /// An expired PSK should fall back to the manifest registration path.
    #[tokio::test]
    async fn bootstrap_expired_psk_falls_back_to_manifest() {
        let response_body = fake_register_response_bytes(true);

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(response_body)
            .expect(1)
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        // Seed ActorStore with an expired PSK.
        let manifest = fake_manifest();
        let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
        let store = ActorStore::open(&storage_path).await.unwrap();

        let expired_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(10); // Expired 10 seconds ago.
        store
            .kv_set("hyper:psk:token", b"expired-psk")
            .await
            .unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expired_at.to_le_bytes())
            .await
            .unwrap();

        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1, test_service_spec(), test_acl())
            .await;

        mock.assert_async().await;
        assert!(
            result.is_ok(),
            "Manifest registration should succeed after PSK expiration, got: {:?}",
            result.err()
        );
    }

    /// AIS errors should propagate as HyperError::AisBootstrapFailed.
    #[tokio::test]
    async fn bootstrap_ais_error_propagates() {
        use actr_protocol::{ErrorResponse, RegisterResponse, register_response};

        let error_resp = RegisterResponse {
            result: Some(register_response::Result::Error(ErrorResponse {
                code: 403,
                message: "manufacturer not trusted".to_string(),
            })),
        }
        .encode_to_vec();

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(error_resp)
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        let manifest = fake_manifest();
        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1, test_service_spec(), test_acl())
            .await;

        assert!(
            matches!(result, Err(HyperError::AisBootstrapFailed(_))),
            "AIS errors should propagate as AisBootstrapFailed, got: {:?}",
            result
        );
    }
}
