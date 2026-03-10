//! Actor 隔离存储实现
//!
//! 每个 Actor 拥有独立的 SQLite 数据库文件，由 Hyper 命名空间解析器确定路径。
//! 所有读写操作均限定在此命名空间内，Actor 无法访问其它 Actor 的数据。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing::{debug, error, info};

use crate::error::{HyperError, HyperResult};

/// KV 批量操作枚举
pub enum KvOp {
    Set { key: String, value: Vec<u8> },
    Delete { key: String },
}

/// Actor 隔离存储句柄
///
/// 每个 Actor 拥有独立的 SQLite 数据库文件，路径由 Hyper 的命名空间解析器决定。
/// 所有读写操作都限定在此命名空间内，Actor 无法访问其它 Actor 的数据。
///
/// rusqlite 连接非 Send，通过 `Arc<Mutex<Connection>>` 包装以支持跨线程共享，
/// 所有阻塞 I/O 通过 `tokio::task::spawn_blocking` 卸载到阻塞线程池。
#[derive(Clone)]
pub struct ActorStore {
    /// 共享 SQLite 连接（rusqlite 不是 Send，用 Mutex 保护）
    conn: Arc<Mutex<rusqlite::Connection>>,
    /// 数据库文件路径（仅用于日志/调试）
    namespace: PathBuf,
}

impl ActorStore {
    /// 打开或创建 Actor 的 SQLite 数据库
    ///
    /// 首次调用时自动建表。父目录不存在时自动创建。
    pub async fn open(db_path: &Path) -> HyperResult<Self> {
        let db_path = db_path.to_path_buf();

        // 确保父目录存在
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                HyperError::Storage(format!(
                    "无法创建存储目录 `{}`: {e}",
                    parent.display()
                ))
            })?;
        }

        let namespace = db_path.clone();

        // rusqlite 是同步 API，通过 spawn_blocking 在阻塞线程池中执行
        let conn = tokio::task::spawn_blocking(move || -> HyperResult<rusqlite::Connection> {
            let conn = rusqlite::Connection::open(&db_path).map_err(|e| {
                error!(
                    path = %db_path.display(),
                    error = %e,
                    "打开 SQLite 数据库失败"
                );
                HyperError::Storage(format!("打开数据库 `{}` 失败: {e}", db_path.display()))
            })?;

            // 开启 WAL 模式，提升并发读性能
            conn.execute_batch("PRAGMA journal_mode=WAL;").map_err(|e| {
                HyperError::Storage(format!("设置 WAL 模式失败: {e}"))
            })?;

            // 建表（idempotent）
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS kv_store (
                    key        TEXT PRIMARY KEY NOT NULL,
                    value      BLOB NOT NULL,
                    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                );",
            )
            .map_err(|e| {
                HyperError::Storage(format!("初始化 kv_store 表失败: {e}"))
            })?;

            Ok(conn)
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking 任务失败: {e}")))??;

        info!(
            path = %namespace.display(),
            "ActorStore 已就绪"
        );

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            namespace,
        })
    }

    /// 通用 KV 存储：写入或更新一个键值对
    pub async fn kv_set(&self, key: &str, value: &[u8]) -> HyperResult<()> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let value = value.to_vec();
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<()> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("获取数据库锁失败: {e}"))
            })?;

            conn.execute(
                "INSERT INTO kv_store (key, value, updated_at)
                 VALUES (?1, ?2, unixepoch())
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                rusqlite::params![key, value],
            )
            .map_err(|e| {
                error!(
                    namespace = %ns.display(),
                    key = %key,
                    error = %e,
                    "kv_set 写入失败"
                );
                HyperError::Storage(format!("kv_set 写入 `{key}` 失败: {e}"))
            })?;

            debug!(namespace = %ns.display(), key = %key, "kv_set 写入成功");
            Ok(())
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking 任务失败: {e}")))??;

        Ok(())
    }

    /// 通用 KV 存储：读取一个键的值，键不存在时返回 None
    pub async fn kv_get(&self, key: &str) -> HyperResult<Option<Vec<u8>>> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<Option<Vec<u8>>> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("获取数据库锁失败: {e}"))
            })?;

            let result = conn.query_row(
                "SELECT value FROM kv_store WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get::<_, Vec<u8>>(0),
            );

            match result {
                Ok(value) => {
                    debug!(namespace = %ns.display(), key = %key, "kv_get 命中");
                    Ok(Some(value))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    debug!(namespace = %ns.display(), key = %key, "kv_get 未命中");
                    Ok(None)
                }
                Err(e) => {
                    error!(
                        namespace = %ns.display(),
                        key = %key,
                        error = %e,
                        "kv_get 读取失败"
                    );
                    Err(HyperError::Storage(format!("kv_get 读取 `{key}` 失败: {e}")))
                }
            }
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking 任务失败: {e}")))?
    }

    /// 通用 KV 存储：删除一个键，返回是否实际删除了记录
    pub async fn kv_delete(&self, key: &str) -> HyperResult<bool> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<bool> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("获取数据库锁失败: {e}"))
            })?;

            let affected = conn
                .execute(
                    "DELETE FROM kv_store WHERE key = ?1",
                    rusqlite::params![key],
                )
                .map_err(|e| {
                    error!(
                        namespace = %ns.display(),
                        key = %key,
                        error = %e,
                        "kv_delete 失败"
                    );
                    HyperError::Storage(format!("kv_delete 删除 `{key}` 失败: {e}"))
                })?;

            let deleted = affected > 0;
            debug!(namespace = %ns.display(), key = %key, deleted, "kv_delete 执行");
            Ok(deleted)
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking 任务失败: {e}")))?
    }

    /// 列出所有 key，可通过 prefix 参数过滤前缀
    pub async fn kv_list_keys(&self, prefix: Option<&str>) -> HyperResult<Vec<String>> {
        let conn = Arc::clone(&self.conn);
        let prefix = prefix.map(|s| s.to_string());
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<Vec<String>> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("获取数据库锁失败: {e}"))
            })?;

            let keys = if let Some(ref pfx) = prefix {
                // 用 LIKE 模式匹配前缀，转义通配符
                let pattern = format!("{}%", pfx.replace('%', "\\%").replace('_', "\\_"));
                let mut stmt = conn
                    .prepare("SELECT key FROM kv_store WHERE key LIKE ?1 ESCAPE '\\' ORDER BY key")
                    .map_err(|e| HyperError::Storage(format!("准备 SQL 失败: {e}")))?;
                let rows = stmt
                    .query_map(rusqlite::params![pattern], |row| row.get::<_, String>(0))
                    .map_err(|e| HyperError::Storage(format!("查询 key 列表失败: {e}")))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| HyperError::Storage(format!("读取 key 行失败: {e}")))?
            } else {
                let mut stmt = conn
                    .prepare("SELECT key FROM kv_store ORDER BY key")
                    .map_err(|e| HyperError::Storage(format!("准备 SQL 失败: {e}")))?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| HyperError::Storage(format!("查询 key 列表失败: {e}")))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| HyperError::Storage(format!("读取 key 行失败: {e}")))?
            };

            debug!(
                namespace = %ns.display(),
                count = keys.len(),
                prefix = ?prefix,
                "kv_list_keys 查询完成"
            );
            Ok(keys)
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking 任务失败: {e}")))?
    }

    /// 批量操作（原子事务）
    ///
    /// 所有操作在同一个事务中执行，任意一步失败则全部回滚。
    pub async fn kv_batch(&self, ops: Vec<KvOp>) -> HyperResult<()> {
        let conn = Arc::clone(&self.conn);
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<()> {
            let mut conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("获取数据库锁失败: {e}"))
            })?;

            let tx = conn.transaction().map_err(|e| {
                HyperError::Storage(format!("开启事务失败: {e}"))
            })?;

            for op in &ops {
                match op {
                    KvOp::Set { key, value } => {
                        tx.execute(
                            "INSERT INTO kv_store (key, value, updated_at)
                             VALUES (?1, ?2, unixepoch())
                             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                            rusqlite::params![key, value],
                        )
                        .map_err(|e| {
                            error!(
                                namespace = %ns.display(),
                                key = %key,
                                error = %e,
                                "kv_batch set 操作失败"
                            );
                            HyperError::Storage(format!("kv_batch set `{key}` 失败: {e}"))
                        })?;
                        debug!(namespace = %ns.display(), key = %key, "kv_batch set");
                    }
                    KvOp::Delete { key } => {
                        tx.execute(
                            "DELETE FROM kv_store WHERE key = ?1",
                            rusqlite::params![key],
                        )
                        .map_err(|e| {
                            error!(
                                namespace = %ns.display(),
                                key = %key,
                                error = %e,
                                "kv_batch delete 操作失败"
                            );
                            HyperError::Storage(format!("kv_batch delete `{key}` 失败: {e}"))
                        })?;
                        debug!(namespace = %ns.display(), key = %key, "kv_batch delete");
                    }
                }
            }

            tx.commit().map_err(|e| {
                error!(namespace = %ns.display(), error = %e, "kv_batch 事务提交失败");
                HyperError::Storage(format!("kv_batch 事务提交失败: {e}"))
            })?;

            debug!(
                namespace = %ns.display(),
                ops_count = ops.len(),
                "kv_batch 事务提交成功"
            );
            Ok(())
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking 任务失败: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn open_test_store(dir: &TempDir) -> ActorStore {
        let db_path = dir.path().join("test.db");
        ActorStore::open(&db_path).await.unwrap()
    }

    #[tokio::test]
    async fn kv_set_and_get() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store.kv_set("hello", b"world").await.unwrap();
        let val = store.kv_get("hello").await.unwrap();
        assert_eq!(val, Some(b"world".to_vec()));
    }

    #[tokio::test]
    async fn kv_get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let val = store.kv_get("nonexistent").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn kv_delete_removes_key() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store.kv_set("key1", b"value1").await.unwrap();
        let deleted = store.kv_delete("key1").await.unwrap();
        assert!(deleted, "应返回 true 表示实际删除了记录");

        let val = store.kv_get("key1").await.unwrap();
        assert_eq!(val, None, "删除后 get 应返回 None");
    }

    #[tokio::test]
    async fn kv_delete_nonexistent_returns_false() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let deleted = store.kv_delete("ghost").await.unwrap();
        assert!(!deleted, "删除不存在的键应返回 false");
    }

    #[tokio::test]
    async fn kv_list_keys_returns_all() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store.kv_set("b", b"2").await.unwrap();
        store.kv_set("a", b"1").await.unwrap();
        store.kv_set("c", b"3").await.unwrap();

        let keys = store.kv_list_keys(None).await.unwrap();
        assert_eq!(keys, vec!["a", "b", "c"], "应按字典序返回所有 key");
    }

    #[tokio::test]
    async fn kv_list_keys_prefix_filter() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store.kv_set("prefix:a", b"1").await.unwrap();
        store.kv_set("prefix:b", b"2").await.unwrap();
        store.kv_set("other:c", b"3").await.unwrap();

        let keys = store.kv_list_keys(Some("prefix:")).await.unwrap();
        assert_eq!(keys, vec!["prefix:a", "prefix:b"]);
    }

    #[tokio::test]
    async fn kv_batch_atomic() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store.kv_set("existing", b"old").await.unwrap();

        store
            .kv_batch(vec![
                KvOp::Set {
                    key: "new_key".to_string(),
                    value: b"new_value".to_vec(),
                },
                KvOp::Set {
                    key: "existing".to_string(),
                    value: b"updated".to_vec(),
                },
                KvOp::Delete {
                    key: "existing".to_string(),
                },
            ])
            .await
            .unwrap();

        // new_key 应存在
        let val = store.kv_get("new_key").await.unwrap();
        assert_eq!(val, Some(b"new_value".to_vec()));

        // existing 在批量中先更新后删除，最终应不存在
        let val = store.kv_get("existing").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn data_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("persist.db");

        {
            let store = ActorStore::open(&db_path).await.unwrap();
            store.kv_set("persistent_key", b"persistent_value").await.unwrap();
        }

        // 重新打开同一文件
        let store2 = ActorStore::open(&db_path).await.unwrap();
        let val = store2.kv_get("persistent_key").await.unwrap();
        assert_eq!(
            val,
            Some(b"persistent_value".to_vec()),
            "重新打开数据库后数据应持久化"
        );
    }
}
