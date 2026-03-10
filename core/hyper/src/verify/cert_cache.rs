//! 生产模式 MFR 公钥缓存
//!
//! `MfrCertCache` 按需从 AIS `GET /mfr/{name}/verifying_key` 获取 manufacturer
//! 的 Ed25519 公钥，并在本地缓存（TTL 1 小时）。
//!
//! 内部使用 `std::sync::RwLock`（非 tokio），因为：
//! - 缓存读写均为极短的内存操作，不会阻塞 tokio executor
//! - 提供同步读路径，供 `PackageVerifier::resolve_mfr_pubkey` 直接调用

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::VerifyingKey;

use crate::error::{HyperError, HyperResult};

/// MFR 公钥缓存条目
struct CacheEntry {
    key: VerifyingKey,
    fetched_at: Instant,
}

/// 生产模式 MFR Ed25519 公钥缓存
///
/// 从 AIS 端点按需获取 manufacturer 公钥，缓存 TTL 默认为 1 小时。
/// 使用 `Arc<MfrCertCache>` 跨任务共享。
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

    /// 仅从内存缓存中查找，不触发 HTTP 拉取（同步）
    ///
    /// 用于 `PackageVerifier::resolve_mfr_pubkey` 同步路径，
    /// 调用前需保证已通过 `get_or_fetch` 预热。
    pub fn get_from_cache(&self, manufacturer: &str) -> Option<VerifyingKey> {
        let cache = self.cache.read().expect("cert_cache read lock poisoned");
        cache.get(manufacturer).and_then(|entry| {
            if entry.fetched_at.elapsed() < self.ttl {
                Some(entry.key)
            } else {
                None
            }
        })
    }

    /// 获取指定 manufacturer 的 Ed25519 验证公钥
    ///
    /// 优先读缓存（未过期），miss 时从 AIS 拉取并更新缓存。
    pub async fn get_or_fetch(&self, manufacturer: &str) -> HyperResult<VerifyingKey> {
        // 快路径：读缓存
        if let Some(key) = self.get_from_cache(manufacturer) {
            tracing::debug!(manufacturer, "MFR 公钥缓存命中");
            return Ok(key);
        }

        tracing::debug!(manufacturer, "MFR 公钥缓存未命中，从 AIS 拉取");

        // 慢路径：HTTP 拉取
        let key = self.fetch_from_ais(manufacturer).await?;

        // 写入缓存（brief blocking lock，只做 HashMap 插入）
        {
            let mut cache = self.cache.write().expect("cert_cache write lock poisoned");
            cache.insert(
                manufacturer.to_string(),
                CacheEntry {
                    key,
                    fetched_at: Instant::now(),
                },
            );
        }

        tracing::info!(manufacturer, "MFR 公钥已从 AIS 获取并缓存");
        Ok(key)
    }

    /// 从 AIS `GET /mfr/{manufacturer}/verifying_key` 获取公钥
    async fn fetch_from_ais(&self, manufacturer: &str) -> HyperResult<VerifyingKey> {
        let url = format!("{}/mfr/{}/verifying_key", self.ais_endpoint, manufacturer);
        tracing::debug!(url, "从 AIS 拉取 MFR 公钥");

        let resp = self.http.get(&url).send().await.map_err(|e| {
            HyperError::UntrustedManufacturer(format!(
                "获取 MFR 公钥失败（{manufacturer}）: {e}"
            ))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                manufacturer,
                status = status.as_u16(),
                body,
                "AIS 返回非 2xx，MFR 公钥获取失败"
            );
            return Err(HyperError::UntrustedManufacturer(format!(
                "AIS 拒绝提供 MFR 公钥（{manufacturer}），status={status}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct VerifyingKeyResp {
            /// Base64 编码的 Ed25519 verifying key（32 字节）
            public_key: String,
        }

        let body: VerifyingKeyResp = resp.json().await.map_err(|e| {
            HyperError::UntrustedManufacturer(format!(
                "解析 MFR 公钥响应失败（{manufacturer}）: {e}"
            ))
        })?;

        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(&body.public_key)
            .map_err(|e| {
                HyperError::UntrustedManufacturer(format!(
                    "MFR 公钥 base64 解码失败（{manufacturer}）: {e}"
                ))
            })?;

        let key_arr: [u8; 32] = key_bytes.try_into().map_err(|v: Vec<u8>| {
            HyperError::UntrustedManufacturer(format!(
                "MFR 公钥长度不正确（{manufacturer}），期望 32 字节，实际 {} 字节",
                v.len()
            ))
        })?;

        VerifyingKey::from_bytes(&key_arr).map_err(|e| {
            HyperError::UntrustedManufacturer(format!(
                "MFR 公钥格式无效（{manufacturer}）: {e}"
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
        let key_b64 = base64::engine::general_purpose::STANDARD
            .encode(verifying_key.to_bytes());

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/mfr/test-mfr/verifying_key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"public_key":"{key_b64}"}}"#))
            .expect(1) // 只调用一次，第二次走缓存
            .create_async()
            .await;

        let cache = MfrCertCache::new(server.url());

        // 第一次 miss → 调用 HTTP
        let k1 = cache.get_or_fetch("test-mfr").await.unwrap();
        // 第二次 hit → 不调用 HTTP
        let k2 = cache.get_or_fetch("test-mfr").await.unwrap();

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
        let result = cache.get_or_fetch("unknown-mfr").await;

        assert!(
            matches!(result, Err(HyperError::UntrustedManufacturer(_))),
            "404 应返回 UntrustedManufacturer，实际: {result:?}"
        );
    }
}
