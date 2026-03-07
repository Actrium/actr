//! AisKeyCache - AIS signing 公钥本地缓存
//!
//! actor 注册时从 RegisterOk 获得当前 AIS signing 公钥，缓存于此。
//! 验签时按 key_id 查找；miss 时通过 signaling 拉取并写入缓存。
//! 公钥无需保密，缓存策略简单：按 key_id 永久保留（key_id 单调递增，条目极少）。

use crate::error::{ActorResult, ActrError};
use crate::wire::SignalingClient;
use actr_protocol::{AIdCredential, ActrId};
use ed25519_dalek::VerifyingKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// AIS Ed25519 signing 公钥缓存
///
/// 线程安全，通过 `Arc<AisKeyCache>` 共享使用。
/// 公钥按 key_id 永久存储；key_id 由 AIS 单调递增分配，实际条目数极少。
pub struct AisKeyCache {
    cache: RwLock<HashMap<u32, VerifyingKey>>,
}

impl AisKeyCache {
    /// 创建新的空缓存，返回 `Arc` 包装以便共享
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// 注册或续期时调用，将 AIS signing 公钥写入缓存
    ///
    /// `pubkey_bytes` 必须是 32 字节的 Ed25519 原始公钥。
    /// 若 key_id 已存在则覆盖（正常情况下不应发生，保持幂等）。
    pub async fn seed(&self, key_id: u32, pubkey_bytes: &[u8]) -> ActorResult<()> {
        let verifying_key = VerifyingKey::from_bytes(
            pubkey_bytes
                .try_into()
                .map_err(|_| ActrError::Internal("signing pubkey 必须为 32 字节".to_string()))?,
        )
        .map_err(|e| ActrError::Internal(format!("signing pubkey 无效: {e}")))?;

        self.cache.write().await.insert(key_id, verifying_key);
        tracing::debug!(key_id, "AisKeyCache: 写入公钥");
        Ok(())
    }

    /// 按 key_id 获取公钥；本地命中直接返回，miss 时通过 signaling 拉取并缓存
    ///
    /// 拉取失败视为不可恢复错误，由调用方决定是否重试。
    pub async fn get_or_fetch(
        &self,
        key_id: u32,
        actor_id: &ActrId,
        credential: &AIdCredential,
        signaling: &dyn SignalingClient,
    ) -> ActorResult<VerifyingKey> {
        // 先持读锁尝试命中，避免不必要的写锁竞争
        {
            let cache = self.cache.read().await;
            if let Some(key) = cache.get(&key_id) {
                tracing::trace!(key_id, "AisKeyCache: 命中缓存");
                return Ok(*key);
            }
        }

        // 缓存未命中，通过 signaling 拉取
        tracing::debug!(key_id, "AisKeyCache: 缓存未命中，向 signaling 拉取公钥");
        let (returned_key_id, pubkey_bytes) = signaling
            .get_signing_key(actor_id.clone(), credential.clone(), key_id)
            .await
            .map_err(|e| {
                tracing::warn!(key_id, error = ?e, "AisKeyCache: 拉取公钥失败");
                ActrError::Internal(format!("拉取 signing 公钥失败: {e:?}"))
            })?;

        let verifying_key = VerifyingKey::from_bytes(
            pubkey_bytes
                .as_slice()
                .try_into()
                .map_err(|_| ActrError::Internal("拉取到的 signing pubkey 必须为 32 字节".to_string()))?,
        )
        .map_err(|e| ActrError::Internal(format!("拉取到的 signing pubkey 无效: {e}")))?;

        self.cache.write().await.insert(returned_key_id, verifying_key);
        tracing::debug!(key_id = returned_key_id, "AisKeyCache: 已缓存从 signaling 获取的公钥");

        Ok(verifying_key)
    }
}
