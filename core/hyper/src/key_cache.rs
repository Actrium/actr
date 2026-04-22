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
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    /// Generate a deterministic Ed25519 key pair from a fixed seed (no rand_core feature needed)
    fn test_signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn verifying_key_bytes(seed: u8) -> [u8; 32] {
        *test_signing_key(seed).verifying_key().as_bytes()
    }

    // ─── mock KeyFetcher ──────────────────────────────────────────────────────

    struct MockFetcher {
        response: Option<(u32, Vec<u8>)>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl MockFetcher {
        fn ok(key_id: u32, bytes: Vec<u8>) -> Self {
            Self {
                response: Some((key_id, bytes)),
                calls: Default::default(),
            }
        }
        fn err() -> Self {
            Self {
                response: None,
                calls: Default::default(),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl KeyFetcher for MockFetcher {
        async fn fetch_key(&self, _key_id: u32) -> HyperResult<(u32, Vec<u8>)> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match &self.response {
                Some(r) => Ok(r.clone()),
                None => Err(HyperError::AisBootstrapFailed("mock fetch error".into())),
            }
        }
    }

    // ─── seed ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn seed_valid_32_bytes_succeeds() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &verifying_key_bytes(1)).await.is_ok());
    }

    #[tokio::test]
    async fn seed_31_bytes_returns_error() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &[0u8; 31]).await.is_err());
    }

    #[tokio::test]
    async fn seed_33_bytes_returns_error() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &[0u8; 33]).await.is_err());
    }

    #[tokio::test]
    async fn seed_empty_returns_error() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &[]).await.is_err());
    }

    #[tokio::test]
    async fn seed_twice_same_key_id_is_idempotent() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(2);
        cache.seed(1, &bytes).await.unwrap();
        cache.seed(1, &bytes).await.unwrap(); // should not error
        let mock = MockFetcher::err();
        let result = cache.get_or_fetch(1, &mock).await;
        assert!(result.is_ok());
        assert_eq!(
            mock.calls(),
            0,
            "seeded key should hit cache, no fetch triggered"
        );
    }

    // ─── get_or_fetch: cache hit ─────────────────────────────────────────────

    #[tokio::test]
    async fn cache_hit_does_not_call_fetcher() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(3);
        cache.seed(1, &bytes).await.unwrap();

        let mock = MockFetcher::err();
        let result = cache.get_or_fetch(1, &mock).await;
        assert!(result.is_ok());
        assert_eq!(mock.calls(), 0);
    }

    #[tokio::test]
    async fn cache_hit_returns_correct_key() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(4);
        cache.seed(7, &bytes).await.unwrap();

        let mock = MockFetcher::err();
        let key = cache.get_or_fetch(7, &mock).await.unwrap();
        assert_eq!(key.as_bytes(), &bytes);
    }

    // ─── get_or_fetch: cache miss ────────────────────────────────────────────

    #[tokio::test]
    async fn cache_miss_calls_fetcher_and_caches_result() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(5);
        let mock = MockFetcher::ok(5, bytes.to_vec());

        let result = cache.get_or_fetch(5, &mock).await;
        assert!(result.is_ok());
        assert_eq!(mock.calls(), 1);

        // second call should hit cache, no further fetch
        let mock2 = MockFetcher::err();
        let result2 = cache.get_or_fetch(5, &mock2).await;
        assert!(result2.is_ok());
        assert_eq!(mock2.calls(), 0, "second call should hit cache");
    }

    #[tokio::test]
    async fn cache_miss_fetcher_failure_returns_error() {
        let cache = AisKeyCache::new();
        let mock = MockFetcher::err();
        let result = cache.get_or_fetch(9, &mock).await;
        assert!(result.is_err());
        assert_eq!(mock.calls(), 1);
    }

    #[tokio::test]
    async fn cache_miss_fetcher_returns_31_byte_pubkey_returns_error() {
        let cache = AisKeyCache::new();
        let mock = MockFetcher::ok(3, vec![0u8; 31]);
        let result = cache.get_or_fetch(3, &mock).await;
        assert!(result.is_err(), "invalid pubkey length should return error");
    }

    #[tokio::test]
    async fn different_key_ids_cached_independently() {
        let cache = AisKeyCache::new();
        cache.seed(1, &verifying_key_bytes(10)).await.unwrap();
        cache.seed(2, &verifying_key_bytes(20)).await.unwrap();

        let mock = MockFetcher::err();
        let k1 = cache.get_or_fetch(1, &mock).await.unwrap();
        let k2 = cache.get_or_fetch(2, &mock).await.unwrap();
        assert_ne!(k1.as_bytes(), k2.as_bytes());
        assert_eq!(mock.calls(), 0);
    }
}
