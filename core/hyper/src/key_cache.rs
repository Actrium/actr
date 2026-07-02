//! AisKeyCache - local cache for AIS signing public keys
//!
//! During actor registration, the current AIS signing public key is obtained from RegisterOk
//! and cached here. Signature verification looks up by key_id; on miss, the `KeyFetcher`
//! fetches and writes into the cache.
//! Public keys need no secrecy; caching strategy is simple: retain permanently by key_id
//! (key_id increases monotonically, very few entries).
//!
//! Unlike the runtime version: depends on `KeyFetcher` trait instead of the full `SignalingClient`,
//! making this module usable independently of the upper communication protocol.

use crate::error::{HyperError, HyperResult};
use async_trait::async_trait;
use ed25519_dalek::VerifyingKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Public key fetching interface
///
/// When a key_id is not found in the local cache, the implementor is responsible for
/// fetching it from a remote source (e.g. AIS/signaling).
/// Returns `(key_id, pubkey_bytes)`, where `pubkey_bytes` must be a 32-byte Ed25519 raw public key.
#[async_trait]
pub(crate) trait KeyFetcher: Send + Sync {
    async fn fetch_key(&self, key_id: u32) -> HyperResult<(u32, Vec<u8>)>;
}

/// AIS Ed25519 signing public key cache
///
/// Thread-safe, shared via `Arc<AisKeyCache>`.
/// Public keys are stored permanently by key_id; key_id is monotonically assigned by AIS,
/// with very few actual entries.
pub(crate) struct AisKeyCache {
    cache: RwLock<HashMap<u32, VerifyingKey>>,
}

impl AisKeyCache {
    /// Create a new empty cache, returned in an `Arc` wrapper for sharing
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Called during registration or renewal to write AIS signing public key into cache
    ///
    /// `pubkey_bytes` must be a 32-byte Ed25519 raw public key.
    /// If key_id already exists, it is overwritten (should not normally occur, kept idempotent).
    pub(crate) async fn seed(&self, key_id: u32, pubkey_bytes: &[u8]) -> HyperResult<()> {
        let verifying_key = VerifyingKey::from_bytes(pubkey_bytes.try_into().map_err(|_| {
            HyperError::InvalidManifest("signing pubkey must be 32 bytes".to_string())
        })?)
        .map_err(|e| HyperError::InvalidManifest(format!("invalid signing pubkey: {e}")))?;

        self.cache.write().await.insert(key_id, verifying_key);
        tracing::debug!(key_id, "AisKeyCache: pubkey written");
        Ok(())
    }

    /// Get public key by key_id; returns directly on local hit, fetches via fetcher on miss
    ///
    /// Fetch failure is treated as an unrecoverable error; the caller decides whether to retry.
    pub(crate) async fn get_or_fetch(
        &self,
        key_id: u32,
        fetcher: &dyn KeyFetcher,
    ) -> HyperResult<VerifyingKey> {
        // Try read lock first to avoid unnecessary write lock contention
        {
            let cache = self.cache.read().await;
            if let Some(key) = cache.get(&key_id) {
                tracing::trace!(key_id, "AisKeyCache: cache hit");
                return Ok(*key);
            }
        }

        // Cache miss, fetch via fetcher
        tracing::debug!(key_id, "AisKeyCache: cache miss, fetching pubkey");
        let (returned_key_id, pubkey_bytes) = fetcher.fetch_key(key_id).await.map_err(|e| {
            tracing::warn!(key_id, error = ?e, "AisKeyCache: pubkey fetch failed");
            e
        })?;

        let verifying_key =
            VerifyingKey::from_bytes(pubkey_bytes.as_slice().try_into().map_err(|_| {
                HyperError::InvalidManifest("fetched signing pubkey must be 32 bytes".to_string())
            })?)
            .map_err(|e| {
                HyperError::InvalidManifest(format!("fetched signing pubkey invalid: {e}"))
            })?;

        self.cache
            .write()
            .await
            .insert(returned_key_id, verifying_key);
        tracing::debug!(
            key_id = returned_key_id,
            "AisKeyCache: cached pubkey fetched from remote"
        );

        Ok(verifying_key)
    }
}

#[cfg(test)]
#[path = "key_cache_tests.rs"]
mod tests;
