//! SQLite 存储后端实现
//!
//! 使用 sqlx 提供原生异步 SQLite 存储支持

use crate::crypto::KeyEncryptor;
use crate::error::{KsError, KsResult};
use crate::storage::backend::KeyStorageBackend;
use crate::storage::config::SqliteConfig;
use crate::types::{KeyPair, KeyRecord};
use async_trait::async_trait;
use base64::prelude::*;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// SQLite 存储后端
#[derive(Clone)]
pub struct SqliteBackend {
    pool: SqlitePool,
    key_ttl: u64,
    encryptor: KeyEncryptor,
}

impl std::fmt::Debug for SqliteBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteBackend")
            .field("key_ttl", &self.key_ttl)
            .field("encryption_enabled", &self.encryptor.is_enabled())
            .finish()
    }
}

impl SqliteBackend {
    /// 创建新的 SQLite 后端实例
    ///
    /// # Arguments
    /// * `_config` - SQLite 配置（暂时保留，未使用）
    /// * `key_ttl` - 密钥有效期（秒），0 表示永不过期
    /// * `encryptor` - 密钥加密器
    /// * `db_path` - 数据库文件存储目录路径（来自 ActrixConfig.sqlite_path）
    pub async fn new(
        _config: &SqliteConfig,
        key_ttl: u64,
        encryptor: KeyEncryptor,
        db_path: &Path,
    ) -> KsResult<Self> {
        let file = db_path.join("ks_keys.db");

        // 创建连接选项并启用 WAL 模式
        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", file.display()))
            .map_err(|e| KsError::Internal(format!("Failed to parse SQLite URL: {e}")))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));

        // 创建连接池
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await
            .map_err(|e| KsError::Internal(format!("Failed to connect to SQLite: {e}")))?;

        let backend = Self {
            pool,
            key_ttl,
            encryptor,
        };

        // 初始化数据库表
        backend.init().await?;

        crate::recording::info!(
            "SQLite storage initialized with sqlx: path={}, key_ttl={}s, encryption={}, WAL mode enabled",
            file.display(),
            key_ttl,
            backend.encryptor.is_enabled()
        );

        Ok(backend)
    }
}

#[async_trait]
impl KeyStorageBackend for SqliteBackend {
    async fn init(&self) -> KsResult<()> {
        // 创建密钥表（secret_key 列存储加密后的 Ed25519 signing key，public_key 存储 verifying key）
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS keys (
                key_id INTEGER PRIMARY KEY AUTOINCREMENT,
                public_key TEXT NOT NULL,
                secret_key TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| KsError::Internal(format!("Failed to create keys table: {e}")))?;

        // 创建索引
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_keys_expires_at ON keys(expires_at)")
            .execute(&self.pool)
            .await
            .map_err(|e| KsError::Internal(format!("Failed to create index: {e}")))?;

        crate::recording::debug!("SQLite tables and indexes initialized");
        Ok(())
    }

    async fn generate_and_store_key(&self) -> KsResult<KeyPair> {
        // 生成 Ed25519 密钥对
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // 编码为 Base64
        let verifying_key_b64 = BASE64_STANDARD.encode(verifying_key.as_bytes());
        let signing_key_b64 = BASE64_STANDARD.encode(signing_key.as_bytes());

        // 加密 signing key（如果启用）
        let encrypted_signing_key = self.encryptor.encrypt(&signing_key_b64)?;

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

        // 插入密钥并返回 ID（public_key 存储 verifying key，secret_key 存储加密的 signing key）
        let result = sqlx::query(
            r#"INSERT INTO keys (public_key, secret_key, created_at, expires_at)
               VALUES (?1, ?2, ?3, ?4)"#,
        )
        .bind(&verifying_key_b64)
        .bind(&encrypted_signing_key)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| KsError::Internal(format!("Failed to insert key: {e}")))?;

        let key_id = result.last_insert_rowid() as u32;

        crate::recording::debug!("Generated Ed25519 signing key with ID: {}", key_id);

        Ok(KeyPair {
            key_id,
            verifying_key: verifying_key_b64,
        })
    }

    async fn get_public_key(&self, key_id: u32) -> KsResult<Option<String>> {
        let result = sqlx::query_as::<_, (String,)>("SELECT public_key FROM keys WHERE key_id = ?")
            .bind(key_id as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                KsError::Internal(format!(
                    "Failed to query public key for key_id {key_id}: {e}"
                ))
            })?;

        if let Some((public_key,)) = result {
            crate::recording::debug!("Found verifying key for key_id: {}", key_id);
            Ok(Some(public_key))
        } else {
            crate::recording::debug!("No verifying key found for key_id: {}", key_id);
            Ok(None)
        }
    }

    async fn sign(&self, key_id: u32, message: &[u8]) -> KsResult<Vec<u8>> {
        // 从数据库读取加密的 signing key
        let result = sqlx::query_as::<_, (String,)>("SELECT secret_key FROM keys WHERE key_id = ?")
            .bind(key_id as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                KsError::Internal(format!(
                    "Failed to query signing key for key_id {key_id}: {e}"
                ))
            })?;

        let encrypted_signing_key = match result {
            Some((key,)) => key,
            None => {
                crate::recording::warn!("Signing key not found for key_id: {}", key_id);
                return Err(KsError::NotFound(format!("Key not found: {key_id}")));
            }
        };

        // 解密 signing key
        let signing_key_b64 = self.encryptor.decrypt(&encrypted_signing_key)?;

        // Base64 解码
        let signing_key_bytes = BASE64_STANDARD
            .decode(&signing_key_b64)
            .map_err(|e| KsError::Crypto(format!("Failed to decode signing key: {e}")))?;

        let signing_key_array: [u8; 32] = signing_key_bytes.try_into().map_err(|_| {
            KsError::Crypto("Signing key must be exactly 32 bytes".to_string())
        })?;

        // 重建 SigningKey 并签名
        let signing_key = SigningKey::from_bytes(&signing_key_array);
        let signature = signing_key.sign(message);

        crate::recording::debug!("Signed message with key_id: {}", key_id);

        Ok(signature.to_bytes().to_vec())
    }

    async fn get_key_record(&self, key_id: u32) -> KsResult<Option<KeyRecord>> {
        let result = sqlx::query_as::<_, (i64, String, i64, i64)>(
            "SELECT key_id, public_key, created_at, expires_at FROM keys WHERE key_id = ?",
        )
        .bind(key_id as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            KsError::Internal(format!(
                "Failed to query key record for key_id {key_id}: {e}"
            ))
        })?;

        if let Some((key_id_db, public_key, created_at, expires_at)) = result {
            crate::recording::debug!("Found key record for key_id: {}", key_id);
            Ok(Some(KeyRecord {
                key_id: key_id_db as u32,
                public_key,
                created_at: created_at as u64,
                expires_at: expires_at as u64,
            }))
        } else {
            crate::recording::debug!("No key record found for key_id: {}", key_id);
            Ok(None)
        }
    }

    async fn get_key_count(&self) -> KsResult<u32> {
        let result = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM keys")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| KsError::Internal(format!("Failed to get key count: {e}")))?;

        Ok(result.0 as u32)
    }

    async fn cleanup_expired_keys(&self, tolerance_seconds: u64) -> KsResult<u32> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let cutoff = now - tolerance_seconds as i64;

        let result = sqlx::query("DELETE FROM keys WHERE expires_at > 0 AND expires_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(|e| KsError::Internal(format!("Failed to cleanup expired keys: {e}")))?;

        let deleted = result.rows_affected() as u32;
        if deleted > 0 {
            crate::recording::debug!(
                "Cleaned up {} expired keys (tolerance {}s)",
                deleted,
                tolerance_seconds
            );
        }

        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn create_test_backend(path: &Path) -> SqliteBackend {
        let config = SqliteConfig {};
        SqliteBackend::new(
            &config,
            3600,
            crate::crypto::KeyEncryptor::no_encryption(),
            path,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_sqlite_init() {
        let temp_dir = tempdir().unwrap();
        let backend = create_test_backend(temp_dir.path()).await;
        let count = backend.get_key_count().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_generate_and_query() {
        let temp_dir = tempdir().unwrap();
        let backend = create_test_backend(temp_dir.path()).await;

        // 生成密钥
        let key_pair = backend.generate_and_store_key().await.unwrap();
        assert!(key_pair.key_id > 0);
        assert!(!key_pair.verifying_key.is_empty());

        // 验证 verifying key 为 32 字节（base64 解码后）
        let vk_bytes = BASE64_STANDARD.decode(&key_pair.verifying_key).unwrap();
        assert_eq!(vk_bytes.len(), 32, "Ed25519 verifying key must be 32 bytes");

        // 查询公钥
        let public_key = backend.get_public_key(key_pair.key_id).await.unwrap();
        assert_eq!(public_key, Some(key_pair.verifying_key.clone()));

        // 查询完整记录
        let record = backend.get_key_record(key_pair.key_id).await.unwrap();
        assert!(record.is_some());
        let record = record.unwrap();
        assert_eq!(record.key_id, key_pair.key_id);
        assert_eq!(record.public_key, key_pair.verifying_key);
    }

    #[tokio::test]
    async fn test_sign() {
        let temp_dir = tempdir().unwrap();
        let backend = create_test_backend(temp_dir.path()).await;

        let key_pair = backend.generate_and_store_key().await.unwrap();
        let message = b"hello world";

        let signature = backend.sign(key_pair.key_id, message).await.unwrap();
        assert_eq!(signature.len(), 64, "Ed25519 signature must be 64 bytes");

        // 验证签名
        let vk_bytes = BASE64_STANDARD.decode(&key_pair.verifying_key).unwrap();
        let vk_array: [u8; 32] = vk_bytes.try_into().unwrap();
        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&vk_array).unwrap();
        let sig_array: [u8; 64] = signature.try_into().unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_array);
        use ed25519_dalek::Verifier;
        assert!(verifying_key.verify(message, &sig).is_ok(), "Signature must be valid");
    }

    #[tokio::test]
    async fn test_sign_nonexistent_key() {
        let temp_dir = tempdir().unwrap();
        let backend = create_test_backend(temp_dir.path()).await;

        let result = backend.sign(999, b"message").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KsError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_query_nonexistent_key() {
        let temp_dir = tempdir().unwrap();
        let backend = create_test_backend(temp_dir.path()).await;

        let public_key = backend.get_public_key(999).await.unwrap();
        assert_eq!(public_key, None);

        let record = backend.get_key_record(999).await.unwrap();
        assert_eq!(record, None);
    }

    #[tokio::test]
    async fn test_key_count() {
        let temp_dir = tempdir().unwrap();
        let backend = create_test_backend(temp_dir.path()).await;

        assert_eq!(backend.get_key_count().await.unwrap(), 0);

        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 1);

        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_cleanup_expired_keys() {
        let temp_dir = tempdir().unwrap();

        // 创建 TTL 为 1 秒的后端
        let config = SqliteConfig {};
        let backend = SqliteBackend::new(
            &config,
            1,
            crate::crypto::KeyEncryptor::no_encryption(),
            temp_dir.path(),
        )
        .await
        .unwrap();

        // 生成密钥
        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 1);

        // 等待过期
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // 清理过期密钥（tolerance=0 表示过期即删）
        let cleaned = backend.cleanup_expired_keys(0).await.unwrap();
        assert_eq!(cleaned, 1);
        assert_eq!(backend.get_key_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_zero_ttl_never_expires() {
        let temp_dir = tempdir().unwrap();

        // TTL 为 0（永不过期）
        let config = SqliteConfig {};
        let backend = SqliteBackend::new(
            &config,
            0,
            crate::crypto::KeyEncryptor::no_encryption(),
            temp_dir.path(),
        )
        .await
        .unwrap();

        backend.generate_and_store_key().await.unwrap();
        assert_eq!(backend.get_key_count().await.unwrap(), 1);

        // 清理不应删除永不过期的密钥
        let cleaned = backend.cleanup_expired_keys(0).await.unwrap();
        assert_eq!(cleaned, 0);
        assert_eq!(backend.get_key_count().await.unwrap(), 1);
    }
}
