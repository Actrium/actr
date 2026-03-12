//! Native platform provider (filesystem + SQLite)

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, info, warn};

use actr_platform_traits::{CryptoProvider, KvStore, PlatformError, PlatformProvider};

use crate::crypto::NativeCryptoProvider;

/// Native platform provider backed by filesystem and SQLite.
pub struct NativePlatformProvider {
    crypto: Arc<NativeCryptoProvider>,
}

impl NativePlatformProvider {
    pub fn new() -> Self {
        Self {
            crypto: Arc::new(NativeCryptoProvider),
        }
    }
}

impl Default for NativePlatformProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PlatformProvider for NativePlatformProvider {
    async fn open_kv_store(&self, namespace: &str) -> Result<Arc<dyn KvStore>, PlatformError> {
        let path = std::path::Path::new(namespace);
        let store = actr_hyper::ActorStore::open(path)
            .await
            .map_err(|e| PlatformError::Storage(format!("failed to open ActorStore: {e}")))?;
        debug!(namespace, "native KV store opened");
        Ok(Arc::new(store))
    }

    async fn load_or_create_instance_id(&self, data_dir: &str) -> Result<String, PlatformError> {
        let id_file = std::path::Path::new(data_dir).join(".hyper-instance-id");

        if id_file.exists() {
            let id = tokio::fs::read_to_string(&id_file)
                .await
                .map_err(|e| PlatformError::Io(format!("failed to read instance_id: {e}")))?;
            let id = id.trim().to_string();
            if !id.is_empty() {
                return Ok(id);
            }
            warn!("instance_id file is empty; generating a new one");
        }

        let new_id = uuid::Uuid::new_v4().to_string();
        tokio::fs::write(&id_file, &new_id)
            .await
            .map_err(|e| PlatformError::Io(format!("failed to write instance_id: {e}")))?;
        info!(instance_id = %new_id, "generated new instance_id");
        Ok(new_id)
    }

    async fn ensure_dir(&self, path: &str) -> Result<(), PlatformError> {
        tokio::fs::create_dir_all(path)
            .await
            .map_err(|e| PlatformError::Io(format!("failed to create directory `{path}`: {e}")))?;
        Ok(())
    }

    fn crypto(&self) -> Arc<dyn CryptoProvider> {
        self.crypto.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn ensure_dir_creates_nested() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a/b/c");
        let provider = NativePlatformProvider::new();

        provider
            .ensure_dir(nested.to_str().unwrap())
            .await
            .unwrap();
        assert!(nested.exists());
    }

    #[tokio::test]
    async fn instance_id_stable_across_calls() {
        let dir = TempDir::new().unwrap();
        let provider = NativePlatformProvider::new();
        let dir_str = dir.path().to_str().unwrap();

        let id1 = provider.load_or_create_instance_id(dir_str).await.unwrap();
        let id2 = provider.load_or_create_instance_id(dir_str).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn kv_store_roundtrip() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let provider = NativePlatformProvider::new();

        let store = provider
            .open_kv_store(db_path.to_str().unwrap())
            .await
            .unwrap();

        store.set("key", b"value").await.unwrap();
        let val = store.get("key").await.unwrap();
        assert_eq!(val, Some(b"value".to_vec()));
    }
}
