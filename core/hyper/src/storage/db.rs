//! Actor isolated storage implementation
//!
//! Each Actor has an independent SQLite database file, with the path determined
//! by Hyper's namespace resolver.
//! All read/write operations are confined to this namespace; an Actor cannot access another Actor's data.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing::{debug, error, info};

use crate::error::{HyperError, HyperResult};

/// KV batch operation enum
pub enum KvOp {
    Set { key: String, value: Vec<u8> },
    Delete { key: String },
}

/// Actor isolated storage handle
///
/// Each Actor has an independent SQLite database file, with the path determined
/// by Hyper's namespace resolver.
/// All read/write operations are confined to this namespace; an Actor cannot access another Actor's data.
///
/// The rusqlite connection is not Send; wrapped in `Arc<Mutex<Connection>>` for cross-thread sharing.
/// All blocking I/O is offloaded to the blocking thread pool via `tokio::task::spawn_blocking`.
#[derive(Clone)]
pub struct ActorStore {
    /// Shared SQLite connection (rusqlite is not Send, protected by Mutex)
    conn: Arc<Mutex<rusqlite::Connection>>,
    /// Database file path (for logging/debugging only)
    namespace: PathBuf,
}

impl ActorStore {
    /// Open or create an Actor's SQLite database
    ///
    /// Automatically creates the table on first call. Parent directory is created automatically if missing.
    pub async fn open(db_path: &Path) -> HyperResult<Self> {
        let db_path = db_path.to_path_buf();

        // ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                HyperError::Storage(format!(
                    "failed to create storage directory `{}`: {e}",
                    parent.display()
                ))
            })?;
        }

        let namespace = db_path.clone();

        // rusqlite is a sync API, execute in blocking thread pool via spawn_blocking
        let conn = tokio::task::spawn_blocking(move || -> HyperResult<rusqlite::Connection> {
            let conn = rusqlite::Connection::open(&db_path).map_err(|e| {
                error!(
                    path = %db_path.display(),
                    error = %e,
                    "failed to open SQLite database"
                );
                HyperError::Storage(format!(
                    "failed to open database `{}`: {e}",
                    db_path.display()
                ))
            })?;

            // enable WAL mode for improved concurrent read performance
            conn.execute_batch("PRAGMA journal_mode=WAL;")
                .map_err(|e| HyperError::Storage(format!("failed to set WAL mode: {e}")))?;

            // create table (idempotent)
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS kv_store (
                    key        TEXT PRIMARY KEY NOT NULL,
                    value      BLOB NOT NULL,
                    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                );",
            )
            .map_err(|e| {
                HyperError::Storage(format!("failed to initialize kv_store table: {e}"))
            })?;

            Ok(conn)
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking task failed: {e}")))??;

        info!(
            path = %namespace.display(),
            "ActorStore ready"
        );

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            namespace,
        })
    }

    /// Generic KV storage: write or update a key-value pair
    pub async fn kv_set(&self, key: &str, value: &[u8]) -> HyperResult<()> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let value = value.to_vec();
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<()> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("failed to acquire database lock: {e}"))
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
                    "kv_set write failed"
                );
                HyperError::Storage(format!("kv_set write `{key}` failed: {e}"))
            })?;

            debug!(namespace = %ns.display(), key = %key, "kv_set write succeeded");
            Ok(())
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking task failed: {e}")))??;

        Ok(())
    }

    /// Generic KV storage: read a key's value, returns None if the key does not exist
    pub async fn kv_get(&self, key: &str) -> HyperResult<Option<Vec<u8>>> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<Option<Vec<u8>>> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("failed to acquire database lock: {e}"))
            })?;

            let result = conn.query_row(
                "SELECT value FROM kv_store WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get::<_, Vec<u8>>(0),
            );

            match result {
                Ok(value) => {
                    debug!(namespace = %ns.display(), key = %key, "kv_get hit");
                    Ok(Some(value))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    debug!(namespace = %ns.display(), key = %key, "kv_get miss");
                    Ok(None)
                }
                Err(e) => {
                    error!(
                        namespace = %ns.display(),
                        key = %key,
                        error = %e,
                        "kv_get read failed"
                    );
                    Err(HyperError::Storage(format!(
                        "kv_get read `{key}` failed: {e}"
                    )))
                }
            }
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking task failed: {e}")))?
    }

    /// Generic KV storage: delete a key, returns whether a record was actually deleted
    pub async fn kv_delete(&self, key: &str) -> HyperResult<bool> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<bool> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("failed to acquire database lock: {e}"))
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
                        "kv_delete failed"
                    );
                    HyperError::Storage(format!("kv_delete `{key}` failed: {e}"))
                })?;

            let deleted = affected > 0;
            debug!(namespace = %ns.display(), key = %key, deleted, "kv_delete executed");
            Ok(deleted)
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking task failed: {e}")))?
    }

    /// List all keys, optionally filtered by prefix
    pub async fn kv_list_keys(&self, prefix: Option<&str>) -> HyperResult<Vec<String>> {
        let conn = Arc::clone(&self.conn);
        let prefix = prefix.map(|s| s.to_string());
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<Vec<String>> {
            let conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("failed to acquire database lock: {e}"))
            })?;

            let keys = if let Some(ref pfx) = prefix {
                // use LIKE pattern matching for prefix, escape wildcards
                let pattern = format!("{}%", pfx.replace('%', "\\%").replace('_', "\\_"));
                let mut stmt = conn
                    .prepare("SELECT key FROM kv_store WHERE key LIKE ?1 ESCAPE '\\' ORDER BY key")
                    .map_err(|e| HyperError::Storage(format!("failed to prepare SQL: {e}")))?;
                let rows = stmt
                    .query_map(rusqlite::params![pattern], |row| row.get::<_, String>(0))
                    .map_err(|e| HyperError::Storage(format!("failed to query key list: {e}")))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| HyperError::Storage(format!("failed to read key row: {e}")))?
            } else {
                let mut stmt = conn
                    .prepare("SELECT key FROM kv_store ORDER BY key")
                    .map_err(|e| HyperError::Storage(format!("failed to prepare SQL: {e}")))?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| HyperError::Storage(format!("failed to query key list: {e}")))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| HyperError::Storage(format!("failed to read key row: {e}")))?
            };

            debug!(
                namespace = %ns.display(),
                count = keys.len(),
                prefix = ?prefix,
                "kv_list_keys query completed"
            );
            Ok(keys)
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking task failed: {e}")))?
    }

    /// Batch operations (atomic transaction)
    ///
    /// All operations execute in a single transaction; if any step fails, all are rolled back.
    pub async fn kv_batch(&self, ops: Vec<KvOp>) -> HyperResult<()> {
        let conn = Arc::clone(&self.conn);
        let ns = self.namespace.clone();

        tokio::task::spawn_blocking(move || -> HyperResult<()> {
            let mut conn = conn.lock().map_err(|e| {
                HyperError::Storage(format!("failed to acquire database lock: {e}"))
            })?;

            let tx = conn.transaction().map_err(|e| {
                HyperError::Storage(format!("failed to begin transaction: {e}"))
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
                                "kv_batch set operation failed"
                            );
                            HyperError::Storage(format!("kv_batch set `{key}` failed: {e}"))
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
                                "kv_batch delete operation failed"
                            );
                            HyperError::Storage(format!("kv_batch delete `{key}` failed: {e}"))
                        })?;
                        debug!(namespace = %ns.display(), key = %key, "kv_batch delete");
                    }
                }
            }

            tx.commit().map_err(|e| {
                error!(namespace = %ns.display(), error = %e, "kv_batch transaction commit failed");
                HyperError::Storage(format!("kv_batch transaction commit failed: {e}"))
            })?;

            debug!(
                namespace = %ns.display(),
                ops_count = ops.len(),
                "kv_batch transaction committed"
            );
            Ok(())
        })
        .await
        .map_err(|e| HyperError::Storage(format!("spawn_blocking task failed: {e}")))?
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
        assert!(
            deleted,
            "should return true indicating a record was actually deleted"
        );

        let val = store.kv_get("key1").await.unwrap();
        assert_eq!(val, None, "get should return None after deletion");
    }

    #[tokio::test]
    async fn kv_delete_nonexistent_returns_false() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let deleted = store.kv_delete("ghost").await.unwrap();
        assert!(!deleted, "deleting a non-existent key should return false");
    }

    #[tokio::test]
    async fn kv_list_keys_returns_all() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store.kv_set("b", b"2").await.unwrap();
        store.kv_set("a", b"1").await.unwrap();
        store.kv_set("c", b"3").await.unwrap();

        let keys = store.kv_list_keys(None).await.unwrap();
        assert_eq!(
            keys,
            vec!["a", "b", "c"],
            "should return all keys in lexicographic order"
        );
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

        // new_key should exist
        let val = store.kv_get("new_key").await.unwrap();
        assert_eq!(val, Some(b"new_value".to_vec()));

        // existing was updated then deleted in the batch, should not exist
        let val = store.kv_get("existing").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn data_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("persist.db");

        {
            let store = ActorStore::open(&db_path).await.unwrap();
            store
                .kv_set("persistent_key", b"persistent_value")
                .await
                .unwrap();
        }

        // reopen the same file
        let store2 = ActorStore::open(&db_path).await.unwrap();
        let val = store2.kv_get("persistent_key").await.unwrap();
        assert_eq!(
            val,
            Some(b"persistent_value".to_vec()),
            "data should persist after reopening the database"
        );
    }
}
