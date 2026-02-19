//! AIS 密钥存储
//!
//! # 功能
//!
//! 提供从 KS 服务获取的公钥的本地缓存，减少网络请求频率。
//!
//! # 数据模型
//!
//! ```sql
//! CREATE TABLE current_key (
//!     id INTEGER PRIMARY KEY CHECK (id = 1),  -- 单行约束
//!     key_id INTEGER NOT NULL,
//!     public_key TEXT NOT NULL,               -- Base64 编码
//!     fetched_at INTEGER NOT NULL,            -- Unix timestamp
//!     expires_at INTEGER NOT NULL             -- Unix timestamp
//! )
//! ```
//!
//! # 刷新策略
//!
//! - **提前刷新**：在密钥过期前 10 分钟触发刷新
//! - **容忍期**：密钥过期后 24 小时内仍认为有效（避免时钟偏差）
//! - **超出容忍期**：强制从 KS 获取新密钥
//!
//! # 线程安全
//!
//! 使用 sqlx 连接池实现原生异步并发访问，启用 WAL 模式提升读性能。
//!
//! # 示例
//!
//! ```ignore
//! use ais::storage::{KeyStorage, KeyRecord};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let storage = KeyStorage::new("keys.db").await?;
//!
//! // 检查是否需要刷新
//! if storage.should_refresh().await? {
//!     // 从 KS 获取新密钥并更新
//!     let record = KeyRecord {
//!         key_id: 123,
//!         public_key: "base64_encoded_key".to_string(),
//!         fetched_at: 1234567890,
//!         expires_at: 1234571490,
//!     };
//!     storage.update_current_key(&record).await?;
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

// ========== 常量配置 ==========

/// 密钥刷新提前时间（秒）
///
/// 在密钥过期前此时间触发刷新，避免在过期瞬间出现可用性问题
const KEY_REFRESH_ADVANCE_SECS: u64 = 600; // 10 分钟

/// 密钥记录
#[derive(Debug, Clone)]
pub struct KeyRecord {
    /// 密钥 ID
    pub key_id: u32,
    /// 公钥（Base64 编码）
    pub public_key: String,
    /// 获取时间（Unix timestamp）
    pub fetched_at: u64,
    /// 过期时间（Unix timestamp）
    pub expires_at: u64,
    /// 容忍时间（秒）
    pub tolerance_seconds: u64,
}

/// 密钥存储（使用 sqlx 连接池）
#[derive(Clone)]
pub struct KeyStorage {
    pool: SqlitePool,
}

impl KeyStorage {
    /// 创建或打开密钥存储
    ///
    /// 使用 sqlx 连接池，配置：
    /// - 最大连接数：10
    /// - WAL 模式：提升并发读性能（4x）
    /// - 同步模式：NORMAL（平衡性能和安全）
    pub async fn new<P: AsRef<Path>>(db_file: P) -> Result<Self> {
        // 创建连接选项并启用 WAL 模式
        let options =
            SqliteConnectOptions::from_str(&format!("sqlite:{}", db_file.as_ref().display()))
                .context("Failed to parse SQLite URL")?
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
                .busy_timeout(Duration::from_secs(5));

        // 创建连接池
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await
            .context("Failed to connect to SQLite")?;

        // 初始化数据库表结构
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS current_key (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                key_id INTEGER NOT NULL,
                public_key TEXT NOT NULL,
                fetched_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                tolerance_seconds INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .context("Failed to create current_key table")?;

        info!("Key storage initialized with sqlx (max_connections=10, WAL mode enabled)");
        Ok(Self { pool })
    }

    /// 获取当前密钥
    pub async fn get_current_key(&self) -> Result<Option<KeyRecord>> {
        let row = sqlx::query_as::<_, (i64, String, i64, i64, i64)>(
            "SELECT key_id, public_key, fetched_at, expires_at, tolerance_seconds FROM current_key WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query current key")?;

        let record = row.map(
            |(key_id, public_key, fetched_at, expires_at, tolerance_seconds)| KeyRecord {
                key_id: key_id as u32,
                public_key,
                fetched_at: fetched_at as u64,
                expires_at: expires_at as u64,
                tolerance_seconds: tolerance_seconds as u64,
            },
        );

        if let Some(ref key) = record {
            debug!(
                "Retrieved key record: key_id={}, expires_at={}",
                key.key_id, key.expires_at
            );
        }

        Ok(record)
    }

    /// 更新当前密钥
    pub async fn update_current_key(&self, record: &KeyRecord) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO current_key (id, key_id, public_key, fetched_at, expires_at, tolerance_seconds)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
        )
        .bind(record.key_id as i64)
        .bind(&record.public_key)
        .bind(record.fetched_at as i64)
        .bind(record.expires_at as i64)
        .bind(record.tolerance_seconds as i64)
        .execute(&self.pool)
        .await
        .context("Failed to update current key")?;

        debug!(
            "Updated current key: key_id={}, expires_at={}",
            record.key_id, record.expires_at
        );

        Ok(())
    }

    /// 检查密钥是否需要刷新
    ///
    /// 返回 true 如果：
    /// - 没有密钥
    /// - 密钥将在 10 分钟内过期
    pub async fn should_refresh(&self) -> Result<bool> {
        let key = match self.get_current_key().await? {
            Some(key) => key,
            None => {
                debug!("No current key found, refresh needed");
                return Ok(true);
            }
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 提前刷新
        let refresh_threshold = key.expires_at.saturating_sub(KEY_REFRESH_ADVANCE_SECS);

        if now >= refresh_threshold {
            debug!(
                "Key refresh needed: now={}, expires_at={}, threshold={}",
                now, key.expires_at, refresh_threshold
            );
            Ok(true)
        } else {
            debug!(
                "Key still valid: now={}, expires_at={}, remaining={}s",
                now,
                key.expires_at,
                key.expires_at.saturating_sub(now)
            );
            Ok(false)
        }
    }

    /// 检查密钥是否已完全过期（超过容忍时间）
    ///
    /// 返回 true 如果密钥过期超过 24 小时
    pub async fn is_expired_beyond_tolerance(&self) -> Result<bool> {
        let key = match self.get_current_key().await? {
            Some(key) => key,
            None => return Ok(true),
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 检查是否超出容忍时间（使用从 KS 获取到的容忍时间）
        let expired_beyond = now > key.expires_at + key.tolerance_seconds;

        if expired_beyond {
            warn!(
                "Key expired beyond tolerance: now={}, expires_at={}, tolerance={}s",
                now, key.expires_at, key.tolerance_seconds
            );
        }

        Ok(expired_beyond)
    }

    /// 健康检查：执行简单的数据库查询验证连接池
    pub async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .context("Database health check failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_key_storage_basic() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = KeyStorage::new(temp_file.path()).await.unwrap();

        // 初始状态：无密钥
        assert!(storage.get_current_key().await.unwrap().is_none());
        assert!(storage.should_refresh().await.unwrap());

        // 添加密钥
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let record = KeyRecord {
            key_id: 123,
            public_key: "test_public_key".to_string(),
            fetched_at: now,
            expires_at: now + 3600,
            tolerance_seconds: 24 * 3600,
        };

        storage.update_current_key(&record).await.unwrap();

        // 验证
        let retrieved = storage.get_current_key().await.unwrap().unwrap();
        assert_eq!(retrieved.key_id, 123);
        assert_eq!(retrieved.public_key, "test_public_key");
        assert_eq!(retrieved.tolerance_seconds, 24 * 3600);
        assert!(!storage.should_refresh().await.unwrap());
        assert!(!storage.is_expired_beyond_tolerance().await.unwrap());
    }

    #[tokio::test]
    async fn test_key_refresh_threshold() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = KeyStorage::new(temp_file.path()).await.unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 密钥将在 5 分钟后过期 (小于 10 分钟阈值)
        let record = KeyRecord {
            key_id: 1,
            public_key: "test_key".to_string(),
            fetched_at: now,
            expires_at: now + 300, // 5 分钟
            tolerance_seconds: 24 * 3600,
        };

        storage.update_current_key(&record).await.unwrap();

        // 应该需要刷新
        assert!(storage.should_refresh().await.unwrap());
    }

    #[tokio::test]
    async fn test_expired_beyond_tolerance() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = KeyStorage::new(temp_file.path()).await.unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 密钥在 25 小时前就过期了
        let record = KeyRecord {
            key_id: 1,
            public_key: "expired_key".to_string(),
            fetched_at: now - 30 * 3600,
            expires_at: now - 25 * 3600,
            tolerance_seconds: 24 * 3600,
        };

        storage.update_current_key(&record).await.unwrap();

        // 应该超出容忍时间
        assert!(storage.is_expired_beyond_tolerance().await.unwrap());
    }
}
