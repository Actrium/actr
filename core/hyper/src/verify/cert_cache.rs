//! Production mode MFR public key cache
//!
//! `MfrCertCache` fetches manufacturer Ed25519 public keys on demand from
//! AIS `GET /mfr/{name}/verifying_key`, caching locally (TTL 1 hour).
//!
//! Uses `std::sync::RwLock` (not tokio) internally because:
//! - Cache reads/writes are extremely short memory operations that won't block the tokio executor
//! - Provides a synchronous read path for `PackageVerifier::resolve_mfr_pubkey` to call directly

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::VerifyingKey;

use crate::error::{HyperError, HyperResult};

/// MFR public key cache entry
struct CacheEntry {
    key: VerifyingKey,
    fetched_at: Instant,
}

/// Production mode MFR Ed25519 public key cache
///
/// Fetches manufacturer public keys on demand from the AIS endpoint, cache TTL defaults to 1 hour.
/// Shared across tasks via `Arc<MfrCertCache>`.
pub struct MfrCertCache {
    ais_endpoint: String,
    http: reqwest::Client,
    ttl: Duration,
    cache: RwLock<HashMap<String, CacheEntry>>,
}

impl MfrCertCache {
    pub fn new(ais_endpoint: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            ais_endpoint: ais_endpoint.into(),
            http: reqwest::Client::new(),
            ttl: Duration::from_secs(3600),
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Used in `PackageVerifier::resolve_mfr_pubkey` synchronous path;
    /// caller must ensure the cache has been warmed via `get_or_fetch` beforehand.
    pub fn get_from_cache(&self, manufacturer: &str, key_id: Option<&str>) -> Option<VerifyingKey> {
        let cache_key = match key_id {
            Some(id) => format!("{}:{}", manufacturer, id),
            None => manufacturer.to_string(),
        };
        let cache = self.cache.read().expect("cert_cache read lock poisoned");
        cache.get(&cache_key).and_then(|entry| {
            if entry.fetched_at.elapsed() < self.ttl {
                Some(entry.key)
            } else {
                None
            }
        })
    }

    /// Get the Ed25519 verifying key for the specified manufacturer
    ///
    /// Reads from cache first (if not expired); on miss, fetches from AIS and updates cache.
    pub async fn get_or_fetch(
        &self,
        manufacturer: &str,
        key_id: Option<&str>,
    ) -> HyperResult<VerifyingKey> {
        // fast path: read cache
        if let Some(key) = self.get_from_cache(manufacturer, key_id) {
            tracing::debug!(manufacturer, ?key_id, "MFR pubkey cache hit");
            return Ok(key);
        }

        tracing::debug!(
            manufacturer,
            ?key_id,
            "MFR pubkey cache miss, fetching from AIS"
        );

        // slow path: HTTP fetch
        let key = self.fetch_from_ais(manufacturer, key_id).await?;

        // write to cache (brief blocking lock, just a HashMap insert)
        let cache_key = match key_id {
            Some(id) => format!("{}:{}", manufacturer, id),
            None => manufacturer.to_string(),
        };
        {
            let mut cache = self.cache.write().expect("cert_cache write lock poisoned");
            cache.insert(
                cache_key,
                CacheEntry {
                    key,
                    fetched_at: Instant::now(),
                },
            );
        }

        tracing::info!(
            manufacturer,
            ?key_id,
            "MFR pubkey fetched from AIS and cached"
        );
        Ok(key)
    }

    /// Fetch public key from AIS `GET /mfr/{manufacturer}/verifying_key`
    async fn fetch_from_ais(
        &self,
        manufacturer: &str,
        key_id: Option<&str>,
    ) -> HyperResult<VerifyingKey> {
        let url = if let Some(id) = key_id {
            format!(
                "{}/mfr/{}/verifying_key?key_id={}",
                self.ais_endpoint, manufacturer, id
            )
        } else {
            format!("{}/mfr/{}/verifying_key", self.ais_endpoint, manufacturer)
        };
        tracing::debug!(url, "fetching MFR pubkey from AIS");

        let resp = self.http.get(&url).send().await.map_err(|e| {
            HyperError::UntrustedManufacturer(format!(
                "failed to fetch MFR pubkey ({manufacturer}): {e}"
            ))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                manufacturer,
                status = status.as_u16(),
                body,
                "AIS returned non-2xx, MFR pubkey fetch failed"
            );
            return Err(HyperError::UntrustedManufacturer(format!(
                "AIS refused to provide MFR pubkey ({manufacturer}), status={status}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct VerifyingKeyResp {
            /// Base64-encoded Ed25519 verifying key (32 bytes)
            public_key: String,
        }

        let body: VerifyingKeyResp = resp.json().await.map_err(|e| {
            HyperError::UntrustedManufacturer(format!(
                "failed to parse MFR pubkey response ({manufacturer}): {e}"
            ))
        })?;

        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(&body.public_key)
            .map_err(|e| {
                HyperError::UntrustedManufacturer(format!(
                    "MFR pubkey base64 decode failed ({manufacturer}): {e}"
                ))
            })?;

        let key_arr: [u8; 32] = key_bytes.try_into().map_err(|v: Vec<u8>| {
            HyperError::UntrustedManufacturer(format!(
                "MFR pubkey length incorrect ({manufacturer}), expected 32 bytes, got {} bytes",
                v.len()
            ))
        })?;

        VerifyingKey::from_bytes(&key_arr).map_err(|e| {
            HyperError::UntrustedManufacturer(format!(
                "MFR pubkey format invalid ({manufacturer}): {e}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_returns_cached_key_without_http() {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;

        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/mfr/test-mfr/verifying_key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"public_key":"{key_b64}"}}"#))
            .expect(1) // only called once, second time hits cache
            .create_async()
            .await;

        let cache = MfrCertCache::new(server.url());

        // first miss -> calls HTTP
        let k1 = cache.get_or_fetch("test-mfr", None).await.unwrap();
        // second hit -> no HTTP call
        let k2 = cache.get_or_fetch("test-mfr", None).await.unwrap();

        mock.assert_async().await;
        assert_eq!(k1.to_bytes(), k2.to_bytes());
        assert_eq!(k1.to_bytes(), verifying_key.to_bytes());
    }

    #[tokio::test]
    async fn fetch_fails_on_404() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/mfr/unknown-mfr/verifying_key")
            .with_status(404)
            .create_async()
            .await;

        let cache = MfrCertCache::new(server.url());
        let result = cache.get_or_fetch("unknown-mfr", None).await;

        assert!(
            matches!(result, Err(HyperError::UntrustedManufacturer(_))),
            "404 should return UntrustedManufacturer, actual: {result:?}"
        );
    }
}
