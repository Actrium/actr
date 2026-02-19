//! 密钥缓存系统
//!
//! 为验证器提供本地 SQLite 缓存，避免频繁从 KS 服务获取私钥

use crate::aid::credential::error::AidError;
use base64::prelude::*;
use ecies::SecretKey;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};

/// 密钥缓存管理器
#[derive(Debug, Clone)]
pub struct KeyCache {
    pool: SqlitePool,
    last_cleanup_time: Arc<Mutex<u64>>,
}

impl KeyCache {
    /// 创建新的密钥缓存实例
    pub async fn new<P: AsRef<Path>>(cache_db_file: P) -> Result<Self, AidError> {
        // 创建 SQLite 连接池（使用 WAL 模式提升性能）
        let database_url = format!("sqlite:{}", cache_db_file.as_ref().display());
        let options = SqliteConnectOptions::from_str(&database_url)
            .map_err(|e| AidError::DecryptionFailed(format!("Invalid database URL: {e}")))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| {
                AidError::DecryptionFailed(format!("Failed to open cache database: {e}"))
            })?;

        let cache = Self {
            pool,
            last_cleanup_time: Arc::new(Mutex::new(0)),
        };

        // 初始化数据库表
        cache.init_tables().await?;

        info!(
            "Key cache initialized with database: {}",
            cache_db_file.as_ref().display()
        );
        Ok(cache)
    }

    /// 初始化缓存数据库表
    async fn init_tables(&self) -> Result<(), AidError> {
        // 创建密钥缓存表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS key_cache (
                key_id INTEGER PRIMARY KEY,
                secret_key TEXT NOT NULL,
                cached_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                tolerance_seconds INTEGER NOT NULL DEFAULT 0
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AidError::DecryptionFailed(format!("Failed to create cache table: {e}")))?;

        // 创建索引以提高查询性能
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_cache_expires_at ON key_cache(expires_at)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AidError::DecryptionFailed(format!("Failed to create cache index: {e}"))
            })?;

        debug!("Cache database tables initialized");
        Ok(())
    }

    /// 从缓存中获取密钥及元数据
    ///
    /// 返回 (SecretKey, expires_at, tolerance_seconds)
    pub async fn get_cached_key(
        &self,
        key_id: u32,
    ) -> Result<Option<(SecretKey, u64, u64)>, AidError> {
        // 检查是否需要清理过期缓存
        self.maybe_cleanup().await;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = sqlx::query(
            "SELECT secret_key, expires_at, tolerance_seconds FROM key_cache WHERE key_id = ?1",
        )
        .bind(key_id as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Database error when querying cached key {}: {}", key_id, e);
            AidError::DecryptionFailed(format!("Cache query error: {e}"))
        })?;

        match result {
            Some(row) => {
                let secret_key_b64: String = row.try_get("secret_key").map_err(|e| {
                    AidError::DecryptionFailed(format!("Failed to get secret_key column: {e}"))
                })?;
                let expires_at: i64 = row.try_get("expires_at").map_err(|e| {
                    AidError::DecryptionFailed(format!("Failed to get expires_at column: {e}"))
                })?;
                let tolerance_seconds: i64 = row.try_get("tolerance_seconds").map_err(|e| {
                    AidError::DecryptionFailed(format!(
                        "Failed to get tolerance_seconds column: {e}"
                    ))
                })?;

                // 检查密钥是否过期（使用 KS 服务返回的过期时间）
                if expires_at > 0 && (expires_at as u64) <= now {
                    debug!(
                        "Cached key {} expired (KS expires_at: {}), removing from cache",
                        key_id, expires_at
                    );
                    // 删除过期的缓存项
                    let _ = sqlx::query("DELETE FROM key_cache WHERE key_id = ?1")
                        .bind(key_id as i64)
                        .execute(&self.pool)
                        .await;
                    return Ok(None);
                }

                // 解码私钥
                let secret_key_bytes = BASE64_STANDARD.decode(&secret_key_b64).map_err(|e| {
                    AidError::DecryptionFailed(format!("Failed to decode cached key: {e}"))
                })?;

                // SecretKey::parse 需要 &[u8; 32] 类型
                let secret_key_array: [u8; 32] = secret_key_bytes.try_into().map_err(|_| {
                    AidError::DecryptionFailed(
                        "Invalid secret key length, expected 32 bytes".to_string(),
                    )
                })?;

                let secret_key = SecretKey::parse(&secret_key_array).map_err(|e| {
                    AidError::DecryptionFailed(format!("Failed to parse cached key: {e}"))
                })?;

                debug!("Found valid cached key for key_id: {}", key_id);
                Ok(Some((
                    secret_key,
                    expires_at as u64,
                    tolerance_seconds as u64,
                )))
            }
            None => {
                debug!("No cached key found for key_id: {}", key_id);
                Ok(None)
            }
        }
    }

    /// 将密钥存入缓存（使用 KS 返回的过期时间和容忍期）
    pub async fn cache_key(
        &self,
        key_id: u32,
        secret_key: &SecretKey,
        expires_at: u64,
        tolerance_seconds: u64,
    ) -> Result<(), AidError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // 编码私钥为 Base64
        let secret_key_b64 = BASE64_STANDARD.encode(secret_key.serialize());

        // 使用 REPLACE 语句，如果存在则更新，不存在则插入
        sqlx::query(
            "REPLACE INTO key_cache (key_id, secret_key, cached_at, expires_at, tolerance_seconds) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(key_id as i64)
        .bind(&secret_key_b64)
        .bind(now as i64)
        .bind(expires_at as i64)
        .bind(tolerance_seconds as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| AidError::DecryptionFailed(format!("Failed to cache key: {e}")))?;

        debug!(
            "Cached key {} with KS expires_at: {}, tolerance: {}s",
            key_id, expires_at, tolerance_seconds
        );
        Ok(())
    }

    /// 清理过期的缓存密钥
    pub async fn cleanup_expired_keys(&self) -> Result<u32, AidError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // 删除过期的密钥（expires_at > 0 且 < now）
        let result = sqlx::query("DELETE FROM key_cache WHERE expires_at > 0 AND expires_at < ?1")
            .bind(now as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AidError::DecryptionFailed(format!("Failed to cleanup expired keys: {e}"))
            })?;

        let deleted_count = result.rows_affected() as u32;

        if deleted_count > 0 {
            info!("Cleaned up {} expired cached keys", deleted_count);
        }

        Ok(deleted_count)
    }

    /// 检查并触发清理（类似 KS 的清理机制）
    async fn maybe_cleanup(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // 检查是否需要清理（在持有锁之前）
        let should_cleanup = {
            let last_cleanup_time = self.last_cleanup_time.lock().unwrap();
            now - *last_cleanup_time >= 3600
        };

        // 如果距离上次清理超过1小时（3600秒），则进行清理
        if should_cleanup {
            debug!("Triggering cache cleanup after 1 hour interval");

            match self.cleanup_expired_keys().await {
                Ok(count) => {
                    if count > 0 {
                        info!("Cache cleanup: removed {} expired keys", count);
                    }
                }
                Err(e) => {
                    error!("Failed to cleanup expired cache keys: {}", e);
                }
            }

            // 更新最后清理时间
            let mut last_cleanup_time = self.last_cleanup_time.lock().unwrap();
            *last_cleanup_time = now;
        }
    }

    /// 获取缓存中的密钥总数
    pub async fn get_cached_key_count(&self) -> Result<u32, AidError> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM key_cache")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AidError::DecryptionFailed(format!("Failed to count cached keys: {e}")))?;

        let count: i64 = row
            .try_get("count")
            .map_err(|e| AidError::DecryptionFailed(format!("Failed to get count column: {e}")))?;

        Ok(count as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_cache_creation() {
        let temp_dir = tempdir().unwrap();
        let cache_path = temp_dir.path().join("test_cache.db");

        let cache = KeyCache::new(&cache_path).await;
        assert!(cache.is_ok());
    }

    #[tokio::test]
    async fn test_key_caching_and_retrieval() {
        let temp_dir = tempdir().unwrap();
        let cache_path = temp_dir.path().join("test_cache.db");
        let cache = KeyCache::new(&cache_path).await.unwrap();

        // 生成测试密钥
        let (secret_key, _) = ecies::utils::generate_keypair();

        // 缓存密钥（设置1小时后过期，容忍期600秒）
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let tolerance_seconds = 600;
        cache
            .cache_key(1, &secret_key, expires_at, tolerance_seconds)
            .await
            .unwrap();
        assert_eq!(cache.get_cached_key_count().await.unwrap(), 1);

        // 检索密钥
        let cached_result = cache.get_cached_key(1).await.unwrap();
        assert!(cached_result.is_some());

        // 验证密钥一致性
        let (retrieved_key, retrieved_expires_at, retrieved_tolerance) = cached_result.unwrap();
        assert_eq!(secret_key.serialize(), retrieved_key.serialize());
        assert_eq!(expires_at, retrieved_expires_at);
        assert_eq!(tolerance_seconds, retrieved_tolerance);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let temp_dir = tempdir().unwrap();
        let cache_path = temp_dir.path().join("test_cache.db");
        let cache = KeyCache::new(&cache_path).await.unwrap();

        // 生成并缓存密钥（设置1秒后过期）
        let (secret_key, _) = ecies::utils::generate_keypair();
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1;
        cache
            .cache_key(1, &secret_key, expires_at, 0)
            .await
            .unwrap();

        // 等待过期
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // 检索应该返回 None（过期了）
        let cached_key = cache.get_cached_key(1).await.unwrap();
        assert!(cached_key.is_none());
    }

    #[tokio::test]
    async fn test_cache_cleanup() {
        let temp_dir = tempdir().unwrap();
        let cache_path = temp_dir.path().join("test_cache.db");
        let cache = KeyCache::new(&cache_path).await.unwrap();

        // 缓存密钥（设置1秒后过期）
        let (secret_key, _) = ecies::utils::generate_keypair();
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1;
        cache
            .cache_key(1, &secret_key, expires_at, 0)
            .await
            .unwrap();
        assert_eq!(cache.get_cached_key_count().await.unwrap(), 1);

        // 等待过期
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // 手动清理
        let cleaned = cache.cleanup_expired_keys().await.unwrap();
        assert_eq!(cleaned, 1);
        assert_eq!(cache.get_cached_key_count().await.unwrap(), 0);
    }
}
