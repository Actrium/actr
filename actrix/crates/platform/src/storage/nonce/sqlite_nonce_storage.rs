//! SQLite Nonce 存储实现
//!
//! 提供基于 SQLite 的 Nonce 存储功能实现，使用 sqlx 提供异步支持

use anyhow::Result;
use nonce_auth::NonceError;
use nonce_auth::storage::{NonceEntry, NonceStorage, StorageStats};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use super::db_nonce_entry::DbNonceEntry;

/// A sqlx-based implementation of NonceStorage for nonce-auth
pub struct SqliteNonceStorage {
    pool: Arc<SqlitePool>,
    cleanup_lock: Arc<RwLock<()>>,
}

impl SqliteNonceStorage {
    /// 创建新的 Nonce 存储实例（同步方法，适用于非 async 上下文）
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let db_file = db_path.as_ref().join("nonce.db");

        // 创建新的运行时来初始化（仅用于非 async 上下文）
        let rt = tokio::runtime::Runtime::new()?;
        let pool = rt.block_on(Self::init_pool(&db_file))?;

        Ok(Self {
            pool: Arc::new(pool),
            cleanup_lock: Arc::new(RwLock::new(())),
        })
    }

    /// 创建新的 Nonce 存储实例（异步方法，适用于 async 上下文）
    pub async fn new_async<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let db_file = db_path.as_ref().join("nonce.db");
        let pool = Self::init_pool(&db_file).await?;

        Ok(Self {
            pool: Arc::new(pool),
            cleanup_lock: Arc::new(RwLock::new(())),
        })
    }

    async fn init_pool<P: AsRef<Path>>(db_file: P) -> Result<SqlitePool> {
        let options =
            SqliteConnectOptions::from_str(&format!("sqlite:{}", db_file.as_ref().display()))?
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
                .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await?;

        // 初始化数据库表
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS nonce_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                nonce TEXT NOT NULL,
                context TEXT,
                expires_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(nonce, context)
            )",
        )
        .execute(&pool)
        .await?;

        // 创建索引
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_nonce_context ON nonce_entries(nonce, context)",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_expires_at ON nonce_entries(expires_at)")
            .execute(&pool)
            .await?;

        Ok(pool)
    }

    /// Get a nonce entry from storage
    pub async fn get(
        &self,
        nonce: &str,
        context: Option<&str>,
    ) -> Result<Option<DbNonceEntry>, NonceError> {
        let result = if let Some(ctx) = context {
            sqlx::query_as::<_, (i64, String, Option<String>, i64, i64)>(
                "SELECT id, nonce, context, expires_at, created_at FROM nonce_entries WHERE nonce = ? AND context = ?",
            )
            .bind(nonce)
            .bind(ctx)
            .fetch_optional(&*self.pool)
            .await
        } else {
            sqlx::query_as::<_, (i64, String, Option<String>, i64, i64)>(
                "SELECT id, nonce, context, expires_at, created_at FROM nonce_entries WHERE nonce = ? AND context IS NULL",
            )
            .bind(nonce)
            .fetch_optional(&*self.pool)
            .await
        };

        match result {
            Ok(Some((id, nonce, context, expires_at, created_at))) => Ok(Some(DbNonceEntry {
                id: Some(id),
                nonce,
                context,
                expires_at,
                created_at,
            })),
            Ok(None) => Ok(None),
            Err(e) => Err(NonceError::from_storage_error(e)),
        }
    }
}

#[async_trait::async_trait]
impl NonceStorage for SqliteNonceStorage {
    async fn get(
        &self,
        nonce: &str,
        context: Option<&str>,
    ) -> Result<Option<NonceEntry>, NonceError> {
        let db_entry = self.get(nonce, context).await?;

        // 检查是否过期
        if let Some(entry) = db_entry {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            if entry.expires_at <= now {
                // 已过期，返回 None
                return Ok(None);
            }

            Ok(Some(NonceEntry {
                nonce: entry.nonce,
                context: entry.context,
                created_at: entry.created_at,
            }))
        } else {
            Ok(None)
        }
    }

    async fn set(
        &self,
        nonce: &str,
        context: Option<&str>,
        ttl: Duration,
    ) -> Result<(), NonceError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let expires_at = now + ttl.as_secs() as i64;

        let result = sqlx::query(
            "INSERT INTO nonce_entries (nonce, context, expires_at, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(nonce)
        .bind(context)
        .bind(expires_at)
        .bind(now)
        .execute(&*self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.message().contains("UNIQUE") => {
                Err(NonceError::DuplicateNonce)
            }
            Err(e) => Err(NonceError::from_storage_error(e)),
        }
    }

    async fn exists(&self, nonce: &str, context: Option<&str>) -> Result<bool, NonceError> {
        Ok(self.get(nonce, context).await?.is_some())
    }

    async fn cleanup_expired(&self, current_time: i64) -> Result<usize, NonceError> {
        // 使用锁防止并发清理
        let _lock = self.cleanup_lock.write().await;

        let result = sqlx::query("DELETE FROM nonce_entries WHERE expires_at < ?")
            .bind(current_time)
            .execute(&*self.pool)
            .await
            .map_err(NonceError::from_storage_error)?;

        Ok(result.rows_affected() as usize)
    }

    async fn get_stats(&self) -> Result<StorageStats, NonceError> {
        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM nonce_entries")
            .fetch_one(&*self.pool)
            .await
            .map_err(NonceError::from_storage_error)?;

        Ok(StorageStats {
            total_records: total.0 as usize,
            backend_info: "SQLite (sqlx async)".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_basic_storage() {
        let temp_dir = tempdir().unwrap();
        let storage = SqliteNonceStorage::new_async(temp_dir.path())
            .await
            .unwrap();

        // Set nonce with TTL
        storage
            .set(
                "test_nonce",
                Some("test_context"),
                Duration::from_secs(3600),
            )
            .await
            .unwrap();

        // Get should return the nonce
        let result = storage
            .get("test_nonce", Some("test_context"))
            .await
            .unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().nonce, "test_nonce");

        // Duplicate should fail
        assert!(matches!(
            storage
                .set(
                    "test_nonce",
                    Some("test_context"),
                    Duration::from_secs(3600)
                )
                .await,
            Err(NonceError::DuplicateNonce)
        ));
    }

    #[tokio::test]
    async fn test_cleanup() {
        let temp_dir = tempdir().unwrap();
        let storage = SqliteNonceStorage::new_async(temp_dir.path())
            .await
            .unwrap();

        let before_insert = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        println!("Before insert: {before_insert}");

        // 添加即将过期的 nonce（1 秒TTL）
        storage
            .set("expired", None, Duration::from_secs(1))
            .await
            .unwrap();

        // 添加未过期的 nonce (1小时TTL)
        storage
            .set("valid", None, Duration::from_secs(3600))
            .await
            .unwrap();

        // 等待确保第一个过期（等待 2 秒）
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let after_sleep = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        println!("After sleep: {after_sleep}");
        println!("Time elapsed: {} seconds", after_sleep - before_insert);

        // 通过 NonceStorage trait 调用（会检查过期）
        use nonce_auth::storage::NonceStorage;
        let expired_entry = NonceStorage::get(&storage, "expired", None).await.unwrap();
        println!("Expired entry after trait call: {expired_entry:?}");

        // 验证：过期的应该查不到（trait方法内部会检查过期）
        assert!(
            expired_entry.is_none(),
            "Expected expired nonce to return None"
        );

        let valid_entry = NonceStorage::get(&storage, "valid", None).await.unwrap();
        assert!(valid_entry.is_some(), "Expected valid nonce to be present");
    }

    #[tokio::test]
    async fn test_stats() {
        let temp_dir = tempdir().unwrap();
        let storage = SqliteNonceStorage::new_async(temp_dir.path())
            .await
            .unwrap();

        // 添加一些 nonce
        storage
            .set("n1", None, Duration::from_secs(1))
            .await
            .unwrap();
        storage
            .set("n2", None, Duration::from_secs(3600))
            .await
            .unwrap();

        let stats = storage.get_stats().await.unwrap();
        assert_eq!(stats.total_records, 2);
        assert!(stats.backend_info.contains("SQLite"));
    }
}
