//! Composite platform provider trait.

use std::sync::Arc;

use async_trait::async_trait;

use crate::PlatformError;
use crate::crypto::CryptoProvider;
use crate::storage::KvStore;

/// Composite platform provider — the three OS-level services a Hyper needs.
///
/// Deliberately narrow and noun-shaped: each method answers one question.
///
/// | Method          | Question                                |
/// |-----------------|-----------------------------------------|
/// | `instance_uid`  | "What's my stable ID, across restarts?" |
/// | `secret_store`  | "Where do I keep actor credentials?"    |
/// | `crypto`        | "How do I verify signatures?"           |
///
/// Implementations own their own root (a filesystem dir on native, a localStorage
/// prefix on web) and handle setup internally. Callers never think in terms of
/// filesystem paths or directory creation.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait PlatformProvider: Send + Sync {
    /// Stable instance UID that survives restarts.
    ///
    /// Must return the same value for the lifetime of the underlying storage
    /// (e.g. a given `data_dir` on native, a given localStorage prefix on web).
    async fn instance_uid(&self) -> Result<String, PlatformError>;

    /// Open a namespaced KV store (per-actor credential / PSK storage).
    ///
    /// The namespace is supplied by the caller; the provider decides how to
    /// combine it with its own root to produce a real storage location.
    async fn secret_store(&self, namespace: &str) -> Result<Arc<dyn KvStore>, PlatformError>;

    /// Cryptographic primitives (signature verification, hashing).
    fn crypto(&self) -> Arc<dyn CryptoProvider>;
}
