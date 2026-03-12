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
//! - Runtime lifecycle management (ActrSystem lifecycle for Native and WASM execution bodies)
//!
//! ### Runtime Infrastructure (formerly actr-runtime)
//!
//! - **Actor Lifecycle**: system init, node start/stop (ActrSystem / ActrNode / ActrRef)
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
//! │  Lifecycle Management (ActrSystem → ActrNode → ActrRef)
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
//! routing decisions are made by the ActrSystem running inside the WASM.

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
pub mod ais_client;
#[cfg(not(target_arch = "wasm32"))]
pub mod key_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;
#[cfg(not(target_arch = "wasm32"))]
pub mod storage;

// Runtime infrastructure modules (native-only)
#[cfg(not(target_arch = "wasm32"))]
pub mod actr_ref;
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

// Executor adapter (native-only, WASM/dynclib host)
#[cfg(not(target_arch = "wasm32"))]
pub mod executor;

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
pub use actr_protocol::{ActrId, ActrType};

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

// Runtime core structures
#[cfg(not(target_arch = "wasm32"))]
pub use actr_ref::ActrRef;
#[cfg(not(target_arch = "wasm32"))]
pub use lifecycle::{ActrNode, ActrSystem, CredentialState, NetworkEventHandle};

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

// Executor adapter
#[cfg(not(target_arch = "wasm32"))]
pub use executor::{
    CallExecutorFn, DispatchContext, DispatchResult, ExecutorAdapter, IoResult, PendingCall,
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
    pub use crate::lifecycle::{
        ActrNode, ActrSystem, CompatLockFile, CompatLockManager, CompatibilityCheck,
        DiscoveryResult,
    };

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

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

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

    /// Verify an ActrPackage byte stream and return the verified manifest
    ///
    /// Successful verification means:
    /// - binary_hash matches the recomputed result (package not tampered with)
    /// - MFR signature is valid (from a trusted manufacturer)
    ///
    /// In production mode, the MFR public key is fetched asynchronously first (requires AIS reachability);
    /// in development mode, there are no network calls and verification is fully synchronous.
    pub async fn verify_package(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // production mode: prefetch MFR public key (write to cert_cache), then verify synchronously
        if matches!(&self.inner.config.trust_mode, TrustMode::Production { .. }) {
            if let Some(manufacturer) = quick_extract_manufacturer(bytes) {
                debug!(manufacturer, "production mode: prefetching MFR public key");
                self.inner.verifier.prefetch_mfr_cert(&manufacturer).await?;
            }
        }
        self.inner.verifier.verify(bytes)
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
    /// Hyper completes registration bootstrap on behalf of the Actor to obtain an ActrId credential.
    /// This credential is then passed to ActrSystem (Native/WASM).
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
    pub async fn bootstrap_credential(
        &self,
        manifest: &PackageManifest,
        ais_endpoint: &str,
        realm_id: u32,
    ) -> HyperResult<Vec<u8>> {
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
                service_spec: None,
                acl: None,
                service: None,
                ws_address: None,
                manifest_json: None,
                mfr_signature: None,
                psk_token: Some(psk_token.into()),
            };
            ais.register_with_psk(req).await?
        } else {
            // Phase 1: first registration, carrying MFR manifest
            info!(
                actr_type = manifest.actr_type_str(),
                "first registration: registering with AIS using MFR manifest"
            );

            // serialize manifest to JSON
            let manifest_json = build_manifest_json(manifest)?;

            let req = RegisterRequest {
                actr_type,
                realm,
                service_spec: None,
                acl: None,
                service: None,
                ws_address: None,
                manifest_json: Some(manifest_json.into()),
                mfr_signature: Some(manifest.signature.clone().into()),
                psk_token: None,
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

        // 6. Serialize AIdCredential and return (credential is a required field, use directly)
        let credential_bytes = ok.credential.encode_to_vec();
        info!(
            actr_type = manifest.actr_type_str(),
            credential_len = credential_bytes.len(),
            "AIS credential bootstrap succeeded"
        );

        Ok(credential_bytes)
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

#[cfg(not(target_arch = "wasm32"))]
/// Serialize a `PackageManifest` into JSON bytes.
///
/// AIS verifies the MFR identity using `manifest_json + mfr_signature`.
/// The JSON includes `signature` encoded as base64 and `binary_hash` encoded as hex, matching the original package format.
fn build_manifest_json(manifest: &PackageManifest) -> HyperResult<Vec<u8>> {
    use base64::Engine;

    let binary_hash_hex: String = manifest
        .binary_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&manifest.signature);

    let json = serde_json::json!({
        "manufacturer": manifest.manufacturer,
        "actr_name": manifest.actr_name,
        "version": manifest.version,
        "binary_hash": binary_hash_hex,
        "capabilities": manifest.capabilities,
        "signature": sig_b64,
    });

    serde_json::to_vec(&json).map_err(|e| {
        HyperError::AisBootstrapFailed(format!("failed to serialize manifest to JSON: {e}"))
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Quickly extract the `manufacturer` field from an `.actr` package, used only to prefetch
/// the MFR public key without full verification.
///
/// Returns `None` when parsing fails or the format is not recognized.
/// The caller then skips prefetch and lets verification report the error.
fn quick_extract_manufacturer(bytes: &[u8]) -> Option<String> {
    if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
        return actr_pack::read_manifest(bytes).ok().map(|m| m.manufacturer);
    }
    None
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
        let result = hyper.verify_package(b"not a wasm file").await;
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    #[tokio::test]
    async fn verify_package_rejects_non_actr_format() {
        let dir = TempDir::new().unwrap();
        let hyper = Hyper::init(dev_config(&dir)).await.unwrap();

        // Non-.actr bytes should return InvalidManifest
        let result = hyper.verify_package(b"\0asm\x01\x00\x00\x00").await;
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

    /// build_manifest_json should output JSON with the required fields.
    #[test]
    fn build_manifest_json_produces_valid_json() {
        let manifest = PackageManifest {
            manufacturer: "acme".to_string(),
            actr_name: "Sensor".to_string(),
            version: "1.0.0".to_string(),
            binary_hash: [0u8; 32],
            capabilities: vec!["storage".to_string()],
            signature: vec![0u8; 64],
        };

        let json_bytes = build_manifest_json(&manifest).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

        assert_eq!(value["manufacturer"], "acme");
        assert_eq!(value["actr_name"], "Sensor");
        assert_eq!(value["version"], "1.0.0");
        // binary_hash should be a 64-character hex string.
        assert_eq!(
            value["binary_hash"].as_str().unwrap().len(),
            64,
            "binary_hash should be a 64-character hex string"
        );
        assert!(value["capabilities"].is_array());
        assert!(
            value["signature"].is_string(),
            "signature should be a base64 string"
        );
    }

    // ─── AIS integration tests (mockito mock server) ────────────────────────

    /// Helper: build a PackageManifest for tests.
    fn fake_manifest() -> PackageManifest {
        PackageManifest {
            manufacturer: "test-mfr".to_string(),
            actr_name: "TestActor".to_string(),
            version: "0.1.0".to_string(),
            binary_hash: [0u8; 32],
            capabilities: vec![],
            signature: vec![0u8; 64],
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
            .bootstrap_credential(&manifest, &server.url(), 1)
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
            .bootstrap_credential(&manifest, &server.url(), 1)
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
            .bootstrap_credential(&manifest, &server.url(), 1)
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
            .bootstrap_credential(&manifest, &server.url(), 1)
            .await;

        assert!(
            matches!(result, Err(HyperError::AisBootstrapFailed(_))),
            "AIS errors should propagate as AisBootstrapFailed, got: {:?}",
            result
        );
    }
}
