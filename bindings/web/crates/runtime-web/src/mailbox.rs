//! IndexedDB-backed Mailbox implementation
//!
//! Implements the `Mailbox` trait from `actr-runtime-mailbox` using
//! browser IndexedDB for persistent message queue storage.
//!
//! ## Design
//!
//! - Each Actor gets its own IndexedDB database: `actr_mailbox_{namespace}`
//! - Messages are stored in a `"messages"` object store keyed by UUID string
//! - A compound index `"status_priority_time"` enables efficient dequeue ordering
//! - `Vec<u8>` fields are stored as `Uint8Array` for zero-copy JS interop
//! - `DateTime<Utc>` is stored as millisecond timestamp (f64)

use std::collections::HashMap;

use actr_runtime_mailbox::{
    Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus, StorageError,
    StorageResult,
};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use js_sys::{Array, Object, Reflect, Uint8Array};
use rexie::{Index, ObjectStore, Rexie, TransactionMode};
use tokio::sync::OnceCell;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;
use wasm_bindgen::JsValue;

/// Maximum number of messages returned per `dequeue()` call.
const DEQUEUE_BATCH_SIZE: usize = 10;

/// IndexedDB database version.
const DB_VERSION: u32 = 1;

/// Object store name for messages.
const STORE_NAME: &str = "messages";

/// Compound index for efficient dequeue: (status, priority_num desc, created_at asc).
const INDEX_STATUS_PRIORITY_TIME: &str = "status_priority_time";

/// IndexedDB-backed Mailbox implementation.
///
/// Each instance manages a lazily-opened database `actr_mailbox_{namespace}`.
/// Messages are persisted as JS objects for direct IndexedDB compatibility.
///
/// # Safety
///
/// `Rexie` contains JS closures that are not `Send`/`Sync` by Rust's type
/// system. In WASM, all execution is single-threaded, so these bounds are
/// trivially satisfied.
pub struct IndexedDbMailbox {
    db_name: String,
    db: OnceCell<Rexie>,
}

// SAFETY: WASM is single-threaded; `Rexie` is never accessed from multiple threads.
unsafe impl Send for IndexedDbMailbox {}
unsafe impl Sync for IndexedDbMailbox {}

/// Intermediate representation for IndexedDB storage.
///
/// All fields are JS-friendly types: strings for UUIDs/enums,
/// `Vec<u8>` (serialized as `Uint8Array`) for binary data,
/// `f64` for timestamps.
#[derive(Debug, Clone)]
struct StoredMessage {
    /// UUID as string (key path)
    id: String,
    /// Sender ActrId bytes
    from: Vec<u8>,
    /// Message payload bytes
    payload: Vec<u8>,
    /// Human-readable priority string: "High" | "Normal"
    priority: String,
    /// Numeric priority for index sorting (higher = more urgent)
    priority_num: u32,
    /// Human-readable status string: "Queued" | "Inflight"
    status: String,
    /// Creation time as milliseconds since Unix epoch
    created_at: f64,
}

impl IndexedDbMailbox {
    /// Create a new IndexedDB mailbox for the given namespace.
    ///
    /// The underlying database is opened lazily on first operation.
    pub fn new(namespace: &str) -> Self {
        let db_name = format!("actr_mailbox_{namespace}");
        debug!(db_name = %db_name, "IndexedDbMailbox created (lazy)");
        Self {
            db_name,
            db: OnceCell::new(),
        }
    }

    /// Open (or reuse) the IndexedDB handle.
    async fn db(&self) -> StorageResult<&Rexie> {
        self.db
            .get_or_try_init(|| async {
                debug!(db_name = %self.db_name, "opening IndexedDB for mailbox");
                Rexie::builder(&self.db_name)
                    .version(DB_VERSION)
                    .add_object_store(
                        ObjectStore::new(STORE_NAME)
                            .key_path("id")
                            .auto_increment(false)
                            .add_index(
                                Index::new(
                                    INDEX_STATUS_PRIORITY_TIME,
                                    "status_priority_time",
                                )
                                .unique(false),
                            ),
                    )
                    .build()
                    .await
                    .map_err(|e| {
                        StorageError::ConnectionError(format!(
                            "failed to open IndexedDB '{}': {e:?}",
                            self.db_name
                        ))
                    })
            })
            .await
    }

    /// Get current timestamp in milliseconds.
    fn now_ms() -> f64 {
        js_sys::Date::now()
    }

    /// Convert `MessagePriority` to a numeric value for index sorting.
    ///
    /// Higher value = higher priority, so `High` sorts before `Normal`
    /// when using a descending cursor or in-memory sort.
    fn priority_to_num(priority: MessagePriority) -> u32 {
        match priority {
            MessagePriority::High => 2,
            MessagePriority::Normal => 1,
        }
    }

    /// Convert `MessagePriority` to its string representation.
    fn priority_to_string(priority: MessagePriority) -> &'static str {
        match priority {
            MessagePriority::High => "High",
            MessagePriority::Normal => "Normal",
        }
    }

    /// Parse `MessagePriority` from a stored string.
    fn priority_from_string(s: &str) -> MessagePriority {
        match s {
            "High" => MessagePriority::High,
            _ => MessagePriority::Normal,
        }
    }

    /// Convert `MessageStatus` to its string representation.
    fn status_to_string(status: MessageStatus) -> &'static str {
        match status {
            MessageStatus::Queued => "Queued",
            MessageStatus::Inflight => "Inflight",
        }
    }

    /// Parse `MessageStatus` from a stored string.
    fn status_from_string(s: &str) -> MessageStatus {
        match s {
            "Inflight" => MessageStatus::Inflight,
            _ => MessageStatus::Queued,
        }
    }

    /// Build a `StoredMessage` from domain types.
    fn to_stored(record: &MessageRecord) -> StoredMessage {
        StoredMessage {
            id: record.id.to_string(),
            from: record.from.clone(),
            payload: record.payload.clone(),
            priority: Self::priority_to_string(record.priority).to_string(),
            priority_num: Self::priority_to_num(record.priority),
            status: Self::status_to_string(record.status).to_string(),
            created_at: record.created_at.timestamp_millis() as f64,
        }
    }

    /// Reconstruct a `MessageRecord` from the stored representation.
    fn from_stored(stored: StoredMessage) -> StorageResult<MessageRecord> {
        let id = Uuid::parse_str(&stored.id).map_err(|e| {
            StorageError::DeserializationError(format!("invalid UUID '{}': {e}", stored.id))
        })?;

        let created_at = Utc
            .timestamp_millis_opt(stored.created_at as i64)
            .single()
            .unwrap_or_else(Utc::now);

        Ok(MessageRecord {
            id,
            from: stored.from,
            payload: stored.payload,
            priority: Self::priority_from_string(&stored.priority),
            status: Self::status_from_string(&stored.status),
            created_at,
        })
    }

    /// Build the compound index key array `[status, priority_num, created_at]`
    /// used by the `status_priority_time` index.
    fn make_index_key(status: &str, priority_num: u32, created_at: f64) -> JsValue {
        let arr = Array::new_with_length(3);
        arr.set(0, JsValue::from_str(status));
        arr.set(1, JsValue::from_f64(priority_num as f64));
        arr.set(2, JsValue::from_f64(created_at));
        arr.into()
    }
}

#[async_trait(?Send)]
impl Mailbox for IndexedDbMailbox {
    async fn enqueue(
        &self,
        from: Vec<u8>,
        payload: Vec<u8>,
        priority: MessagePriority,
    ) -> StorageResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Self::now_ms();
        let created_at = Utc
            .timestamp_millis_opt(now as i64)
            .single()
            .unwrap_or_else(Utc::now);

        let record = MessageRecord {
            id,
            from,
            payload,
            priority,
            status: MessageStatus::Queued,
            created_at,
        };

        let stored = Self::to_stored(&record);
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| {
                StorageError::ConnectionError(format!("failed to create transaction: {e:?}"))
            })?;

        let store = tx.store(STORE_NAME).map_err(|e| {
            StorageError::QueryError(format!("failed to access object store: {e:?}"))
        })?;

        // Build JS object with the compound index key path
        let js_value = Self::build_js_record(&stored)?;

        store.add(&js_value, None).await.map_err(|e| {
            StorageError::QueryError(format!("failed to add message: {e:?}"))
        })?;

        tx.done().await.map_err(|e| {
            StorageError::QueryError(format!("transaction commit failed: {e:?}"))
        })?;

        debug!(
            message_id = %id,
            priority = ?priority,
            "enqueued message"
        );
        Ok(id)
    }

    async fn dequeue(&self) -> StorageResult<Vec<MessageRecord>> {
        let rexie = self.db().await?;

        // Phase 1: Read all records and select Queued messages
        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| {
                StorageError::ConnectionError(format!("failed to create transaction: {e:?}"))
            })?;

        let store = tx.store(STORE_NAME).map_err(|e| {
            StorageError::QueryError(format!("failed to access object store: {e:?}"))
        })?;

        let all_values = store.get_all(None, None).await.map_err(|e| {
            StorageError::QueryError(format!("failed to get all messages: {e:?}"))
        })?;

        tx.done().await.map_err(|e| {
            StorageError::QueryError(format!("read transaction commit failed: {e:?}"))
        })?;

        // Deserialize and filter Queued messages
        let mut queued: Vec<StoredMessage> = Vec::new();
        for js_val in all_values {
            match Self::parse_js_record(&js_val) {
                Ok(stored) => {
                    if stored.status == "Queued" {
                        queued.push(stored);
                    }
                }
                Err(e) => {
                    warn!(error = %e, "skipping unreadable message record");
                }
            }
        }

        // Sort: highest priority first, then oldest first (FIFO within same priority)
        queued.sort_by(|a, b| {
            b.priority_num
                .cmp(&a.priority_num)
                .then_with(|| a.created_at.partial_cmp(&b.created_at).unwrap_or(std::cmp::Ordering::Equal))
        });

        // Take up to batch size
        queued.truncate(DEQUEUE_BATCH_SIZE);

        if queued.is_empty() {
            trace!("dequeue: no queued messages");
            return Ok(Vec::new());
        }

        // Phase 2: Mark selected messages as Inflight
        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| {
                StorageError::ConnectionError(format!("failed to create write transaction: {e:?}"))
            })?;

        let store = tx.store(STORE_NAME).map_err(|e| {
            StorageError::QueryError(format!("failed to access object store: {e:?}"))
        })?;

        let mut results = Vec::with_capacity(queued.len());

        for mut stored in queued {
            // Update status to Inflight
            stored.status = "Inflight".to_string();

            let js_value = Self::build_js_record(&stored)?;
            store.put(&js_value, None).await.map_err(|e| {
                StorageError::QueryError(format!(
                    "failed to update message {} to Inflight: {e:?}",
                    stored.id
                ))
            })?;

            results.push(Self::from_stored(stored)?);
        }

        tx.done().await.map_err(|e| {
            StorageError::QueryError(format!("write transaction commit failed: {e:?}"))
        })?;

        info!(count = results.len(), "dequeued messages marked as Inflight");
        Ok(results)
    }

    async fn ack(&self, message_id: Uuid) -> StorageResult<()> {
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| {
                StorageError::ConnectionError(format!("failed to create transaction: {e:?}"))
            })?;

        let store = tx.store(STORE_NAME).map_err(|e| {
            StorageError::QueryError(format!("failed to access object store: {e:?}"))
        })?;

        let key = JsValue::from_str(&message_id.to_string());
        store.delete(key).await.map_err(|e| {
            StorageError::QueryError(format!("failed to delete message {message_id}: {e:?}"))
        })?;

        tx.done().await.map_err(|e| {
            StorageError::QueryError(format!("transaction commit failed: {e:?}"))
        })?;

        debug!(message_id = %message_id, "acknowledged and deleted message");
        Ok(())
    }

    async fn status(&self) -> StorageResult<MailboxStats> {
        let rexie = self.db().await?;

        let tx = rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| {
                StorageError::ConnectionError(format!("failed to create transaction: {e:?}"))
            })?;

        let store = tx.store(STORE_NAME).map_err(|e| {
            StorageError::QueryError(format!("failed to access object store: {e:?}"))
        })?;

        let all_values = store.get_all(None, None).await.map_err(|e| {
            StorageError::QueryError(format!("failed to get all messages: {e:?}"))
        })?;

        tx.done().await.map_err(|e| {
            StorageError::QueryError(format!("transaction commit failed: {e:?}"))
        })?;

        let mut queued_messages: u64 = 0;
        let mut inflight_messages: u64 = 0;
        let mut queued_by_priority: HashMap<MessagePriority, u64> = HashMap::new();

        for js_val in all_values {
            match Self::parse_js_record(&js_val) {
                Ok(stored) => {
                    let status = Self::status_from_string(&stored.status);
                    let priority = Self::priority_from_string(&stored.priority);

                    match status {
                        MessageStatus::Queued => {
                            queued_messages += 1;
                            *queued_by_priority.entry(priority).or_insert(0) += 1;
                        }
                        MessageStatus::Inflight => {
                            inflight_messages += 1;
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "skipping unreadable record in status scan");
                }
            }
        }

        let stats = MailboxStats {
            queued_messages,
            inflight_messages,
            queued_by_priority,
        };

        debug!(
            queued = stats.queued_messages,
            inflight = stats.inflight_messages,
            "mailbox status"
        );
        Ok(stats)
    }
}

// ---------------------------------------------------------------------------
// JS object construction / parsing helpers
// ---------------------------------------------------------------------------

impl IndexedDbMailbox {
    /// Build a JS object from a `StoredMessage` for IndexedDB storage.
    ///
    /// The object has shape:
    /// ```js
    /// {
    ///   id: "uuid-string",
    ///   from: Uint8Array,
    ///   payload: Uint8Array,
    ///   priority: "High" | "Normal",
    ///   priority_num: 1 | 2,
    ///   status: "Queued" | "Inflight",
    ///   created_at: 1234567890.0,
    ///   status_priority_time: ["Queued", 2, 1234567890.0]
    /// }
    /// ```
    fn build_js_record(stored: &StoredMessage) -> StorageResult<JsValue> {
        let obj = Object::new();

        set_prop(&obj, "id", &JsValue::from_str(&stored.id))?;

        let from_arr = Uint8Array::new_with_length(stored.from.len() as u32);
        from_arr.copy_from(&stored.from);
        set_prop(&obj, "from", &from_arr.into())?;

        let payload_arr = Uint8Array::new_with_length(stored.payload.len() as u32);
        payload_arr.copy_from(&stored.payload);
        set_prop(&obj, "payload", &payload_arr.into())?;

        set_prop(&obj, "priority", &JsValue::from_str(&stored.priority))?;
        set_prop(
            &obj,
            "priority_num",
            &JsValue::from_f64(stored.priority_num as f64),
        )?;
        set_prop(&obj, "status", &JsValue::from_str(&stored.status))?;
        set_prop(
            &obj,
            "created_at",
            &JsValue::from_f64(stored.created_at),
        )?;

        // Compound index key path value
        let index_val =
            Self::make_index_key(&stored.status, stored.priority_num, stored.created_at);
        set_prop(&obj, "status_priority_time", &index_val)?;

        Ok(obj.into())
    }

    /// Parse a JS record back into a `StoredMessage`.
    fn parse_js_record(js_val: &JsValue) -> StorageResult<StoredMessage> {
        let id = get_string_prop(js_val, "id")?;
        let priority = get_string_prop(js_val, "priority")?;
        let status = get_string_prop(js_val, "status")?;

        let priority_num = get_f64_prop(js_val, "priority_num")? as u32;
        let created_at = get_f64_prop(js_val, "created_at")?;

        let from = get_uint8array_prop(js_val, "from")?;
        let payload = get_uint8array_prop(js_val, "payload")?;

        Ok(StoredMessage {
            id,
            from,
            payload,
            priority,
            priority_num,
            status,
            created_at,
        })
    }
}

// ---------------------------------------------------------------------------
// JS interop helpers
// ---------------------------------------------------------------------------

/// Set a property on a JS object.
fn set_prop(obj: &Object, key: &str, value: &JsValue) -> StorageResult<()> {
    Reflect::set(obj, &JsValue::from_str(key), value).map_err(|e| {
        StorageError::SerializationError(format!("Reflect::set '{key}' failed: {e:?}"))
    })?;
    Ok(())
}

/// Get a string property from a JS object.
fn get_string_prop(obj: &JsValue, key: &str) -> StorageResult<String> {
    let val = Reflect::get(obj, &JsValue::from_str(key)).map_err(|e| {
        StorageError::DeserializationError(format!("Reflect::get '{key}' failed: {e:?}"))
    })?;
    val.as_string().ok_or_else(|| {
        StorageError::DeserializationError(format!("property '{key}' is not a string"))
    })
}

/// Get a f64 property from a JS object.
fn get_f64_prop(obj: &JsValue, key: &str) -> StorageResult<f64> {
    let val = Reflect::get(obj, &JsValue::from_str(key)).map_err(|e| {
        StorageError::DeserializationError(format!("Reflect::get '{key}' failed: {e:?}"))
    })?;
    val.as_f64().ok_or_else(|| {
        StorageError::DeserializationError(format!("property '{key}' is not a number"))
    })
}

/// Get a Uint8Array property from a JS object and convert to Vec<u8>.
fn get_uint8array_prop(obj: &JsValue, key: &str) -> StorageResult<Vec<u8>> {
    let val = Reflect::get(obj, &JsValue::from_str(key)).map_err(|e| {
        StorageError::DeserializationError(format!("Reflect::get '{key}' failed: {e:?}"))
    })?;
    let array = Uint8Array::new(&val);
    Ok(array.to_vec())
}
