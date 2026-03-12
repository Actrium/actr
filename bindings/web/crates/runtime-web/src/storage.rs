//! IndexedDB-backed KvStore implementation
//!
//! Each namespace maps to a separate IndexedDB database (`actr_kv_{namespace}`),
//! providing per-Actor storage isolation. Values are stored as JS objects with
//! shape `{ key: String, value: Uint8Array }`.

use actr_platform_traits::{KvOp, KvStore, PlatformError};
use async_trait::async_trait;
use js_sys::{Object, Reflect, Uint8Array};
use rexie::{ObjectStore, Rexie, TransactionMode};
use std::cell::OnceCell;
use tracing::{debug, trace};
use wasm_bindgen::JsValue;

const STORE_NAME: &str = "kv";
const DB_VERSION: u32 = 1;

/// IndexedDB-backed key-value store.
///
/// Each instance owns a lazily-initialized IndexedDB database named
/// `actr_kv_{namespace}`, containing a single object store `"kv"` keyed by the
/// `"key"` property.
///
/// # Safety
///
/// `Rexie` contains JS closures that are not `Send`/`Sync` by Rust's type
/// system. In WASM, all execution is single-threaded, so these bounds are
/// trivially satisfied.
pub struct IndexedDbKvStore {
    db_name: String,
    db: OnceCell<Rexie>,
}

// SAFETY: WASM is single-threaded; `Rexie` is never accessed from multiple threads.
unsafe impl Send for IndexedDbKvStore {}
unsafe impl Sync for IndexedDbKvStore {}

impl std::fmt::Debug for IndexedDbKvStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexedDbKvStore")
            .field("db_name", &self.db_name)
            .field("initialized", &self.db.get().is_some())
            .finish()
    }
}

impl IndexedDbKvStore {
    /// Create a new KV store for the given namespace.
    ///
    /// The underlying IndexedDB database is opened lazily on first access.
    pub fn new(namespace: &str) -> Self {
        let db_name = format!("actr_kv_{namespace}");
        debug!(db_name = %db_name, "IndexedDbKvStore created (lazy)");
        Self {
            db_name,
            db: OnceCell::new(),
        }
    }

    /// Open (or reuse) the IndexedDB handle.
    async fn db(&self) -> Result<&Rexie, PlatformError> {
        if let Some(rexie) = self.db.get() {
            return Ok(rexie);
        }

        debug!(db_name = %self.db_name, "opening IndexedDB");
        let rexie = Rexie::builder(&self.db_name)
            .version(DB_VERSION)
            .add_object_store(ObjectStore::new(STORE_NAME).key_path("key"))
            .build()
            .await
            .map_err(|e| {
                PlatformError::Storage(format!(
                    "failed to open IndexedDB '{}': {e:?}",
                    self.db_name
                ))
            })?;

        // In single-threaded WASM this cannot race, but `set` returns Err if
        // already initialized, which we handle by falling through to `get`.
        let _ = self.db.set(rexie);
        Ok(self.db.get().expect("just initialized"))
    }

    /// Build a JS object `{ key, value }` suitable for the object store.
    fn make_record(key: &str, value: &[u8]) -> Result<JsValue, PlatformError> {
        let obj = Object::new();
        Reflect::set(&obj, &"key".into(), &JsValue::from_str(key))
            .map_err(|e| PlatformError::Storage(format!("Reflect::set key failed: {e:?}")))?;

        let array = Uint8Array::new_with_length(value.len() as u32);
        array.copy_from(value);
        Reflect::set(&obj, &"value".into(), &array.into())
            .map_err(|e| PlatformError::Storage(format!("Reflect::set value failed: {e:?}")))?;

        Ok(obj.into())
    }

    /// Extract the `value` field from a stored JS record into `Vec<u8>`.
    fn extract_value(record: &JsValue) -> Result<Vec<u8>, PlatformError> {
        let js_val = Reflect::get(record, &"value".into())
            .map_err(|e| PlatformError::Storage(format!("Reflect::get value failed: {e:?}")))?;

        let array = Uint8Array::new(&js_val);
        Ok(array.to_vec())
    }

    /// Extract the `key` field from a stored JS record.
    fn extract_key(record: &JsValue) -> Result<String, PlatformError> {
        let js_val = Reflect::get(record, &"key".into())
            .map_err(|e| PlatformError::Storage(format!("Reflect::get key failed: {e:?}")))?;

        js_val
            .as_string()
            .ok_or_else(|| PlatformError::Storage("key field is not a string".into()))
    }
}

#[async_trait(?Send)]
impl KvStore for IndexedDbKvStore {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PlatformError> {
        trace!(key = %key, "KvStore::get");
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| PlatformError::Storage(format!("transaction open failed: {e:?}")))?;
        let store = tx
            .store(STORE_NAME)
            .map_err(|e| PlatformError::Storage(format!("store access failed: {e:?}")))?;

        let result = store
            .get(JsValue::from_str(key))
            .await
            .map_err(|e| PlatformError::Storage(format!("get failed: {e:?}")))?;

        tx.done()
            .await
            .map_err(|e| PlatformError::Storage(format!("transaction commit failed: {e:?}")))?;

        match result {
            Some(record) => {
                let bytes = Self::extract_value(&record)?;
                trace!(key = %key, len = bytes.len(), "KvStore::get hit");
                Ok(Some(bytes))
            }
            None => {
                trace!(key = %key, "KvStore::get miss");
                Ok(None)
            }
        }
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<(), PlatformError> {
        trace!(key = %key, len = value.len(), "KvStore::set");
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| PlatformError::Storage(format!("transaction open failed: {e:?}")))?;
        let store = tx
            .store(STORE_NAME)
            .map_err(|e| PlatformError::Storage(format!("store access failed: {e:?}")))?;

        let record = Self::make_record(key, value)?;
        store
            .put(&record, None)
            .await
            .map_err(|e| PlatformError::Storage(format!("put failed: {e:?}")))?;

        tx.done()
            .await
            .map_err(|e| PlatformError::Storage(format!("transaction commit failed: {e:?}")))?;

        debug!(key = %key, "KvStore::set committed");
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<bool, PlatformError> {
        trace!(key = %key, "KvStore::delete");
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| PlatformError::Storage(format!("transaction open failed: {e:?}")))?;
        let store = tx
            .store(STORE_NAME)
            .map_err(|e| PlatformError::Storage(format!("store access failed: {e:?}")))?;

        // Check existence first so we can return an accurate boolean.
        let existed = store
            .key_exists(JsValue::from_str(key))
            .await
            .map_err(|e| PlatformError::Storage(format!("key_exists failed: {e:?}")))?;

        if existed {
            store
                .delete(JsValue::from_str(key))
                .await
                .map_err(|e| PlatformError::Storage(format!("delete failed: {e:?}")))?;
        }

        tx.done()
            .await
            .map_err(|e| PlatformError::Storage(format!("transaction commit failed: {e:?}")))?;

        if existed {
            debug!(key = %key, "KvStore::delete removed");
        } else {
            trace!(key = %key, "KvStore::delete key absent");
        }
        Ok(existed)
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, PlatformError> {
        trace!(prefix = ?prefix, "KvStore::list_keys");
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| PlatformError::Storage(format!("transaction open failed: {e:?}")))?;
        let store = tx
            .store(STORE_NAME)
            .map_err(|e| PlatformError::Storage(format!("store access failed: {e:?}")))?;

        let all = store
            .get_all(None, None)
            .await
            .map_err(|e| PlatformError::Storage(format!("get_all failed: {e:?}")))?;

        let mut keys = Vec::with_capacity(all.len());
        for record in &all {
            let k = Self::extract_key(record)?;
            if let Some(pfx) = prefix {
                if k.starts_with(pfx) {
                    keys.push(k);
                }
            } else {
                keys.push(k);
            }
        }

        tx.done()
            .await
            .map_err(|e| PlatformError::Storage(format!("transaction commit failed: {e:?}")))?;

        debug!(prefix = ?prefix, count = keys.len(), "KvStore::list_keys done");
        Ok(keys)
    }

    async fn batch(&self, ops: Vec<KvOp>) -> Result<(), PlatformError> {
        if ops.is_empty() {
            return Ok(());
        }
        debug!(count = ops.len(), "KvStore::batch");
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| PlatformError::Storage(format!("transaction open failed: {e:?}")))?;
        let store = tx
            .store(STORE_NAME)
            .map_err(|e| PlatformError::Storage(format!("store access failed: {e:?}")))?;

        for op in &ops {
            match op {
                KvOp::Set { key, value } => {
                    let record = Self::make_record(key, value)?;
                    store
                        .put(&record, None)
                        .await
                        .map_err(|e| PlatformError::Storage(format!("batch put failed: {e:?}")))?;
                    trace!(key = %key, "batch: set");
                }
                KvOp::Delete { key } => {
                    // Silently ignore missing keys within a batch.
                    store
                        .delete(JsValue::from_str(key))
                        .await
                        .map_err(|e| {
                            PlatformError::Storage(format!("batch delete failed: {e:?}"))
                        })?;
                    trace!(key = %key, "batch: delete");
                }
            }
        }

        tx.done()
            .await
            .map_err(|e| PlatformError::Storage(format!("transaction commit failed: {e:?}")))?;

        debug!(count = ops.len(), "KvStore::batch committed");
        Ok(())
    }
}
