//! Platform-agnostic key-value storage trait

use std::sync::Arc;

use async_trait::async_trait;

use crate::PlatformError;

/// Batch operation for KV store
pub enum KvOp {
    Set { key: String, value: Vec<u8> },
    Delete { key: String },
}

/// Platform-agnostic key-value store trait
///
/// Each Actor gets an isolated KV namespace. The backing implementation
/// may be SQLite (native), IndexedDB (web), or any other storage engine.
#[async_trait]
pub trait KvStore: Send + Sync {
    /// Read a key's value, returns `None` if the key does not exist
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PlatformError>;

    /// Write or update a key-value pair
    async fn set(&self, key: &str, value: &[u8]) -> Result<(), PlatformError>;

    /// Delete a key, returns whether a record was actually deleted
    async fn delete(&self, key: &str) -> Result<bool, PlatformError>;

    /// List all keys, optionally filtered by prefix
    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, PlatformError>;

    /// Batch operations (atomic transaction)
    async fn batch(&self, ops: Vec<KvOp>) -> Result<(), PlatformError>;
}

/// Extension: obtain a type-erased clone of a KvStore
pub trait KvStoreClone: KvStore {
    fn clone_box(&self) -> Arc<dyn KvStore>;
}
