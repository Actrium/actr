//! AId Credential 验证器
//!
//! 负责验证和解密 AId Token，专注于验证职责

use super::error::AidError;
use crate::aid::identity_claims::IdentityClaims;
use crate::aid::key_cache::KeyCache;
use crate::config::ks::KsClientConfig;
use actr_protocol::AIdCredential;
use ecies::{SecretKey, decrypt};
use ks::GrpcClient;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error};

/// AId Token 验证器 - 提供静态方法验证和解密 Token
pub struct AIdCredentialValidator {
    key_cache: Arc<KeyCache>,
    ks_client: Arc<RwLock<GrpcClient>>,
}

static VALIDATOR_INSTANCE: OnceCell<Arc<AIdCredentialValidator>> = OnceCell::new();

impl AIdCredentialValidator {
    /// 创建新的验证器实例
    pub async fn new(
        ks_client_config: &KsClientConfig,
        actrix_shared_key: &str,
        sqlite_path: &std::path::Path,
    ) -> Result<Self, AidError> {
        let cache_db_file = sqlite_path.join("ks_cache.db");

        let key_cache = Arc::new(KeyCache::new(cache_db_file).await?);

        // 创建 gRPC 客户端配置
        let grpc_config = ks::GrpcClientConfig {
            endpoint: ks_client_config.endpoint.clone(),
            actrix_shared_key: actrix_shared_key.to_string(),
            timeout_seconds: ks_client_config.timeout_seconds,
            enable_tls: ks_client_config.enable_tls,
            tls_domain: ks_client_config.tls_domain.clone(),
            ca_cert: ks_client_config.ca_cert.clone(),
            client_cert: ks_client_config.client_cert.clone(),
            client_key: ks_client_config.client_key.clone(),
        };

        let grpc_client = GrpcClient::new(&grpc_config).await.map_err(|e| {
            AidError::DecryptionFailed(format!("Failed to create KS gRPC client: {e}"))
        })?;

        let ks_client = Arc::new(RwLock::new(grpc_client));

        Ok(Self {
            key_cache,
            ks_client,
        })
    }

    /// 初始化全局验证器实例
    pub async fn init(
        ks_client_config: &KsClientConfig,
        actrix_shared_key: &str,
        sqlite_path: &std::path::Path,
    ) -> Result<(), AidError> {
        let validator = Self::new(ks_client_config, actrix_shared_key, sqlite_path).await?;
        VALIDATOR_INSTANCE
            .set(Arc::new(validator))
            .map_err(|_| AidError::DecryptionFailed("Validator already initialized".to_string()))?;
        Ok(())
    }

    /// 获取全局验证器实例
    fn get_instance() -> Result<Arc<AIdCredentialValidator>, AidError> {
        VALIDATOR_INSTANCE.get().cloned().ok_or_else(|| {
            AidError::DecryptionFailed(
                "Validator not initialized. Call AIdCredentialValidator::init() first".to_string(),
            )
        })
    }

    /// 检查 credential (解密 + 验证有效性 + 容忍期检测)
    ///
    /// 使用 AIdCredential 进行验证，并返回密钥是否在容忍期
    ///
    /// # Arguments
    /// * `credential` - 来自 actor-rtc-proto 的 AIdCredential
    /// * `realm_id` - 期望的 Realm ID
    ///
    /// # Returns
    /// * `Ok((Claims, in_tolerance_period))` - 验证成功，返回解密后的身份声明和容忍期状态
    /// * `Err(AidError)` - 验证失败，包含具体错误信息
    pub async fn check(
        credential: &AIdCredential,
        realm_id: u32,
    ) -> Result<(IdentityClaims, bool), AidError> {
        let validator = Self::get_instance()?;
        let (secret_key, in_tolerance) = validator
            .get_secret_key_with_tolerance(credential.token_key_id)
            .await?;
        let claims = Self::check_with_key(credential, realm_id, &secret_key)?;
        Ok((claims, in_tolerance))
    }

    /// Synchronously checks a credential (decryption + validity verification)
    ///
    /// This synchronous method is intended for use in sync contexts (such as TURN authentication).
    pub fn check_sync(
        credential: &AIdCredential,
        realm_id: u32,
    ) -> Result<IdentityClaims, AidError> {
        let validator = Self::get_instance()?;

        // Use block_in_place to execute async operations without blocking the entire runtime
        let (secret_key, _in_tolerance) = tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::try_current().map_err(|_| {
                AidError::DecryptionFailed("Not in tokio runtime context".to_string())
            })?;

            handle.block_on(async {
                validator
                    .get_secret_key_with_tolerance(credential.token_key_id)
                    .await
            })
        })?;

        Self::check_with_key(credential, realm_id, &secret_key)
    }

    /// 使用提供的密钥检查 credential (解密 + 验证有效性)
    ///
    /// # Arguments  
    /// * `credential` - 来自 actor-rtc-proto 的 AIdCredential
    /// * `realm_id` - 期望的 Realm ID
    /// * `secret_key` - 用于解密的密钥
    ///
    /// # Returns
    /// * `Ok(Claims)` - 验证成功，返回解密后的身份声明
    /// * `Err(AidError)` - 验证失败，包含具体错误信息
    pub fn check_with_key(
        credential: &AIdCredential,
        realm_id: u32,
        secret_key: &SecretKey,
    ) -> Result<IdentityClaims, AidError> {
        // 将 SecretKey 转换为字节
        let secret_key_bytes = secret_key.serialize();

        // 解密
        let decrypted_bytes = decrypt(&secret_key_bytes, &credential.encrypted_token)
            .map_err(|e| AidError::DecryptionFailed(format!("Decryption error: {e}")))?;

        // 反序列化
        let claims: IdentityClaims = serde_json::from_slice(&decrypted_bytes)
            .map_err(|e| AidError::DecryptionFailed(format!("Deserialization error: {e}")))?;

        // 验证 credential 是否过期
        if claims.is_expired() {
            return Err(AidError::Expired);
        }

        // 验证 realm_id 是否匹配
        if claims.realm_id != realm_id {
            return Err(AidError::DecryptionFailed("Realm ID mismatch".to_string()));
        }

        Ok(claims)
    }

    /// 根据 key_id 获取对应的密钥和容忍期状态
    ///
    /// 实现完整的密钥管理逻辑：
    /// 1. 首先尝试从本地 SQLite 缓存中读取私钥和元数据
    /// 2. 如果缓存中没有或已过期，则从 KS 服务获取
    /// 3. 更新本地缓存
    /// 4. 返回密钥和容忍期状态
    async fn get_secret_key_with_tolerance(
        &self,
        key_id: u32,
    ) -> Result<(SecretKey, bool), AidError> {
        debug!("Fetching secret key for key_id: {}", key_id);

        // 1. 首先尝试从 SQLite 缓存获取
        match self.key_cache.get_cached_key(key_id).await? {
            Some((secret_key, expires_at, tolerance_seconds)) => {
                debug!("Found cached secret key for key_id: {}", key_id);
                // 计算是否在容忍期内
                let in_tolerance = Self::calculate_tolerance_status(expires_at, tolerance_seconds);
                return Ok((secret_key, in_tolerance));
            }
            None => {
                debug!(
                    "No cached key found for key_id: {}, fetching from KS",
                    key_id
                );
            }
        }

        // 2. 从 KS 服务获取密钥、过期时间和容忍期秒数
        let (secret_key, expires_at, tolerance_seconds) = {
            let mut client = self.ks_client.write().await;
            client.fetch_secret_key(key_id).await.map_err(|e| {
                error!("Failed to fetch secret key {} from KS: {}", key_id, e);
                AidError::DecryptionFailed(format!("KS error: {e}"))
            })?
        };

        // 3. 更新 SQLite 缓存（包含过期时间和容忍期秒数）
        if let Err(cache_err) = self
            .key_cache
            .cache_key(key_id, &secret_key, expires_at, tolerance_seconds)
            .await
        {
            // 缓存失败不应该影响主要功能，只记录错误
            error!("Failed to cache secret key {}: {}", key_id, cache_err);
        } else {
            debug!("Successfully cached secret key for key_id: {}", key_id);
        }

        // 4. 计算容忍期状态
        let in_tolerance = Self::calculate_tolerance_status(expires_at, tolerance_seconds);

        Ok((secret_key, in_tolerance))
    }

    /// 计算密钥是否在容忍期内
    fn calculate_tolerance_status(expires_at: u64, tolerance_seconds: u64) -> bool {
        if expires_at == 0 {
            return false; // 永不过期的密钥不在容忍期
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 在容忍期内：已过期但未超过容忍期
        expires_at < now && now <= expires_at + tolerance_seconds
    }
}
