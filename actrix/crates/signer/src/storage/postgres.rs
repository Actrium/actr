//! PostgreSQL 存储后端实现
//!
//! 使用 sqlx 提供 PostgreSQL 存储支持

use crate::error::{SignerError, SignerResult};
use crate::storage::backend::KeyStorageBackend;
use crate::storage::config::PostgresConfig;
use crate::types::{KeyPair, KeyRecord};
use async_trait::async_trait;
use base64::prelude::*;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// PostgreSQL 存储后端
#[derive(Clone)]
pub struct PostgresBackend {
    pool: PgPool,
    key_ttl: u64,
}

impl std::fmt::Debug for PostgresBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresBackend")
            .field("key_ttl", &self.key_ttl)
            .finish()
    }
}

impl PostgresBackend {
    /// 创建新的 PostgreSQL 后端实例
    ///
    /// # Arguments
    /// * `config` - PostgreSQL 配置
    /// * `key_ttl` - 密钥有效期（秒），0 表示永不过期
    pub async fn new(config: &PostgresConfig, key_ttl: u64) -> SignerResult<Self> {
        // 构建连接 URL
        let url = format!(
            "postgres://{}:{}@{}:{}/{}",
            config.username, config.password, config.host, config.port, config.database
        );

        // 创建连接池
        let pool = PgPoolOptions::new()
            .max_connections(config.pool_size)
            .max_lifetime(Duration::from_secs(config.max_lifetime_secs))
            .connect(&url)
            .await
            .map_err(|e| SignerError::Internal(format!("Failed to connect to PostgreSQL: {e}")))?;

        let backend = Self { pool, key_ttl };

        // 初始化数据库表
        backend.init().await?;

        crate::recording::info!(
            "PostgreSQL storage initialized: host={}:{}, db={}, key_ttl={}s",
            config.host,
            config.port,
            config.database,
            key_ttl
        );

        Ok(backend)
    }
}

#[async_trait]
impl KeyStorageBackend for PostgresBackend {
    async fn init(&self) -> SignerResult<()> {
        // 创建密钥表（secret_key 列存储加密后的 Ed25519 signing key，public_key 存储 verifying key）
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS keys (
                key_id SERIAL PRIMARY KEY,
                public_key TEXT NOT NULL,
                secret_key TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                expires_at BIGINT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| SignerError::Internal(format!("Failed to create keys table: {e}")))?;

        // 创建索引以提高过期查询性能
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_keys_expires_at ON keys(expires_at) WHERE expires_at > 0",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| SignerError::Internal(format!("Failed to create index: {e}")))?;

        crate::recording::debug!("PostgreSQL tables and indexes initialized");
        Ok(())
    }

    async fn generate_and_store_key(&self) -> SignerResult<KeyPair> {
        // 生成 Ed25519 密钥对
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // 编码为 Base64
        let verifying_key_b64 = BASE64_STANDARD.encode(verifying_key.as_bytes());
        let signing_key_b64 = BASE64_STANDARD.encode(signing_key.as_bytes());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // 计算过期时间
        let expires_at = if self.key_ttl == 0 {
            0 // 永不过期
        } else {
            now + self.key_ttl as i64
        };

        // 插入密钥并获取自动生成的 key_id（public_key 存储 verifying key，secret_key 存储 signing key）
        let row = sqlx::query_as::<_, (i32,)>(
            r#"
            INSERT INTO keys (public_key, secret_key, created_at, expires_at)
            VALUES ($1, $2, $3, $4)
            RETURNING key_id
            "#,
        )
        .bind(&verifying_key_b64)
        .bind(&signing_key_b64)
        .bind(now)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| SignerError::Internal(format!("Failed to insert key: {e}")))?;

        let key_id = row.0 as u32;

        crate::recording::info!(
            "Generated and stored new Ed25519 key pair in PostgreSQL: key_id={}, expires_at={}",
            key_id,
            expires_at
        );

        Ok(KeyPair {
            key_id,
            verifying_key: verifying_key_b64,
        })
    }

    async fn get_public_key(&self, key_id: u32) -> SignerResult<Option<String>> {
        let result =
            sqlx::query_scalar::<_, String>("SELECT public_key FROM keys WHERE key_id = $1")
                .bind(key_id as i32)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    SignerError::Internal(format!(
                        "Failed to query public key for key_id {key_id}: {e}"
                    ))
                })?;

        if result.is_some() {
            crate::recording::debug!("Found verifying key for key_id: {} in PostgreSQL", key_id);
        } else {
            crate::recording::debug!(
                "No verifying key found for key_id: {} in PostgreSQL",
                key_id
            );
        }

        Ok(result)
    }

    async fn sign(&self, key_id: u32, message: &[u8]) -> SignerResult<Vec<u8>> {
        // 从数据库读取 signing key（PostgreSQL 版本未加密）
        let result =
            sqlx::query_scalar::<_, String>("SELECT secret_key FROM keys WHERE key_id = $1")
                .bind(key_id as i32)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    SignerError::Internal(format!(
                        "Failed to query signing key for key_id {key_id}: {e}"
                    ))
                })?;

        let signing_key_b64 = match result {
            Some(key) => key,
            None => {
                crate::recording::warn!(
                    "Signing key not found for key_id: {} in PostgreSQL",
                    key_id
                );
                return Err(SignerError::NotFound(format!("Key not found: {key_id}")));
            }
        };

        // Base64 解码
        let signing_key_bytes = BASE64_STANDARD
            .decode(&signing_key_b64)
            .map_err(|e| SignerError::Crypto(format!("Failed to decode signing key: {e}")))?;

        let signing_key_array: [u8; 32] = signing_key_bytes
            .try_into()
            .map_err(|_| SignerError::Crypto("Signing key must be exactly 32 bytes".to_string()))?;

        // 重建 SigningKey 并签名
        let signing_key = SigningKey::from_bytes(&signing_key_array);
        let signature = signing_key.sign(message);

        crate::recording::debug!("Signed message with key_id: {} in PostgreSQL", key_id);

        Ok(signature.to_bytes().to_vec())
    }

    async fn get_key_record(&self, key_id: u32) -> SignerResult<Option<KeyRecord>> {
        let result = sqlx::query_as::<_, (i32, String, i64, i64)>(
            "SELECT key_id, public_key, created_at, expires_at FROM keys WHERE key_id = $1",
        )
        .bind(key_id as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            SignerError::Internal(format!(
                "Failed to query key record for key_id {key_id}: {e}"
            ))
        })?;

        match result {
            Some((id, public_key, created_at, expires_at)) => {
                crate::recording::debug!("Found key record for key_id: {} in PostgreSQL", key_id);
                Ok(Some(KeyRecord {
                    key_id: id as u32,
                    public_key,
                    created_at: created_at as u64,
                    expires_at: expires_at as u64,
                }))
            }
            None => {
                crate::recording::debug!(
                    "No key record found for key_id: {} in PostgreSQL",
                    key_id
                );
                Ok(None)
            }
        }
    }

    async fn get_key_count(&self) -> SignerResult<u32> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM keys")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| SignerError::Internal(format!("Failed to get key count: {e}")))?;

        Ok(count as u32)
    }

    async fn cleanup_expired_keys(&self, tolerance_seconds: u64) -> SignerResult<u32> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let cutoff = now - tolerance_seconds as i64;

        let result = sqlx::query("DELETE FROM keys WHERE expires_at > 0 AND expires_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(|e| SignerError::Internal(format!("Failed to cleanup expired keys: {e}")))?;

        let deleted_count = result.rows_affected() as u32;

        if deleted_count > 0 {
            crate::recording::info!(
                "Cleaned up {} expired keys from PostgreSQL (tolerance {}s)",
                deleted_count,
                tolerance_seconds
            );
        }

        Ok(deleted_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_backend() -> PostgresBackend {
        let config = PostgresConfig {
            host: std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string()),
            port: std::env::var("POSTGRES_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5432),
            database: std::env::var("POSTGRES_DB").unwrap_or_else(|_| "ks_test".to_string()),
            username: std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string()),
            password: std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string()),
            pool_size: 5,
            max_lifetime_secs: 3600,
        };

        PostgresBackend::new(&config, 3600).await.unwrap()
    }

    async fn cleanup_test_data(backend: &PostgresBackend) {
        sqlx::query("TRUNCATE TABLE keys RESTART IDENTITY")
            .execute(&backend.pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore] // 需要 PostgreSQL 服务器
    async fn test_postgres_init() {
        let backend = create_test_backend().await;
        cleanup_test_data(&backend).await;

        let count = backend.get_key_count().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    #[ignore] // 需要 PostgreSQL 服务器
    async fn test_generate_and_query() {
        let backend = create_test_backend().await;
        cleanup_test_data(&backend).await;

        // 生成密钥
        let key_pair = backend.generate_and_store_key().await.unwrap();
        assert!(key_pair.key_id > 0);
        assert!(!key_pair.verifying_key.is_empty());

        // 验证 verifying key 为 32 字节
        let vk_bytes = BASE64_STANDARD.decode(&key_pair.verifying_key).unwrap();
        assert_eq!(vk_bytes.len(), 32);

        // 查询公钥
        let public_key = backend.get_public_key(key_pair.key_id).await.unwrap();
        assert_eq!(public_key, Some(key_pair.verifying_key.clone()));

        cleanup_test_data(&backend).await;
    }

    #[tokio::test]
    #[ignore] // 需要 PostgreSQL 服务器
    async fn test_sign() {
        let backend = create_test_backend().await;
        cleanup_test_data(&backend).await;

        let key_pair = backend.generate_and_store_key().await.unwrap();
        let message = b"test message";
        let signature = backend.sign(key_pair.key_id, message).await.unwrap();
        assert_eq!(signature.len(), 64);

        cleanup_test_data(&backend).await;
    }

    #[tokio::test]
    #[ignore] // 需要 PostgreSQL 服务器
    async fn test_query_nonexistent_key() {
        let backend = create_test_backend().await;
        cleanup_test_data(&backend).await;

        let result = backend.get_public_key(99999).await.unwrap();
        assert_eq!(result, None);

        cleanup_test_data(&backend).await;
    }

    #[tokio::test]
    #[ignore] // 需要 PostgreSQL 服务器
    async fn test_key_count() {
        let backend = create_test_backend().await;
        cleanup_test_data(&backend).await;

        assert_eq!(backend.get_key_count().await.unwrap(), 0);

        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 1);

        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 2);

        cleanup_test_data(&backend).await;
    }

    #[tokio::test]
    #[ignore] // 需要 PostgreSQL 服务器
    async fn test_cleanup_expired_keys() {
        let config = PostgresConfig {
            host: std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string()),
            port: std::env::var("POSTGRES_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5432),
            database: std::env::var("POSTGRES_DB").unwrap_or_else(|_| "ks_test".to_string()),
            username: std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string()),
            password: std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string()),
            pool_size: 5,
            max_lifetime_secs: 3600,
        };

        // 创建 TTL 为 1 秒的后端
        let backend = PostgresBackend::new(&config, 1).await.unwrap();
        cleanup_test_data(&backend).await;

        // 生成密钥
        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 1);

        // 等待过期
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // 清理过期密钥
        let cleaned = backend.cleanup_expired_keys(0).await.unwrap();
        assert_eq!(cleaned, 1);
        assert_eq!(backend.get_key_count().await.unwrap(), 0);

        cleanup_test_data(&backend).await;
    }

    #[tokio::test]
    #[ignore] // 需要 PostgreSQL 服务器
    async fn test_zero_ttl_never_expires() {
        let config = PostgresConfig {
            host: std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string()),
            port: std::env::var("POSTGRES_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5432),
            database: std::env::var("POSTGRES_DB").unwrap_or_else(|_| "ks_test".to_string()),
            username: std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string()),
            password: std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string()),
            pool_size: 5,
            max_lifetime_secs: 3600,
        };

        // TTL 为 0（永不过期）
        let backend = PostgresBackend::new(&config, 0).await.unwrap();
        cleanup_test_data(&backend).await;

        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 1);

        // 清理不应删除永不过期的密钥
        let cleaned = backend.cleanup_expired_keys(0).await.unwrap();
        assert_eq!(cleaned, 0);
        assert_eq!(backend.get_key_count().await.unwrap(), 1);

        cleanup_test_data(&backend).await;
    }
}
