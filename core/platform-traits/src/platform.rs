//! Composite platform provider trait

use std::sync::Arc;

use async_trait::async_trait;

use crate::PlatformError;
use crate::crypto::CryptoProvider;
use crate::storage::KvStore;

/// Composite platform provider
///
/// Groups all platform-specific services behind a single injectable interface.
/// `ActrSystem` and `Hyper` accept `Arc<dyn PlatformProvider>` to decouple
/// from concrete native/web implementations.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait PlatformProvider: Send + Sync {
    /// Open a KV store isolated by namespace (each Actor gets its own)
    async fn open_kv_store(&self, namespace: &str) -> Result<Arc<dyn KvStore>, PlatformError>;

    /// Load or generate and persist an instance ID
    async fn load_or_create_instance_id(&self, data_dir: &str) -> Result<String, PlatformError>;

    /// Ensure a directory exists (native: fs::create_dir_all, web: no-op)
    async fn ensure_dir(&self, path: &str) -> Result<(), PlatformError>;

    /// Get the cryptography provider
    fn crypto(&self) -> Arc<dyn CryptoProvider>;
}
