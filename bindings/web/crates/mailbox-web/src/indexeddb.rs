//! IndexedDB implementation of Mailbox using rexie
//!
//! Full implementation using rexie high-level API

use rexie::*;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use web_sys::console;

use crate::{
    Mailbox, MailboxError, MailboxStats, MessagePriority, MessageRecord, MessageStatus, Result,
};

const DB_NAME: &str = "actr_mailbox";
const DB_VERSION: u32 = 1;
const STORE_NAME: &str = "messages";
const INDEX_PRIORITY_TIME: &str = "priority_time";

/// IndexedDB-based Mailbox implementation
pub struct IndexedDbMailbox {
    rexie: Rc<Rexie>,
}

/// Serializable message record for IndexedDB
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMessage {
    id: String,
    from: Vec<u8>,
    payload: Vec<u8>,
    priority: String,
    priority_num: u32,
    status: String,
    created_at: f64,
}

impl IndexedDbMailbox {
    /// Create a new IndexedDB mailbox
    pub async fn new() -> Result<Self> {
        console::log_1(&"Creating IndexedDB Mailbox with rexie".into());

        let rexie = Rexie::builder(DB_NAME)
            .version(DB_VERSION)
            .add_object_store(
                ObjectStore::new(STORE_NAME)
                    .key_path("id")
                    .auto_increment(false)
                    .add_index(Index::new(INDEX_PRIORITY_TIME, "priority_num").unique(false)),
            )
            .build()
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to open DB: {:?}", e)))?;

        console::log_1(&"IndexedDB opened successfully with rexie".into());

        Ok(Self {
            rexie: Rc::new(rexie),
        })
    }

    /// Get current timestamp in milliseconds
    fn now_ms() -> u64 {
        js_sys::Date::now() as u64
    }

    /// Convert MessagePriority to numeric value for sorting
    fn priority_to_num(priority: MessagePriority) -> u32 {
        match priority {
            MessagePriority::High => 3,
            MessagePriority::Normal => 2,
            MessagePriority::Low => 1,
        }
    }

    /// Convert MessagePriority to string
    fn priority_to_string(priority: MessagePriority) -> String {
        match priority {
            MessagePriority::High => "High".to_string(),
            MessagePriority::Normal => "Normal".to_string(),
            MessagePriority::Low => "Low".to_string(),
        }
    }

    /// Parse MessagePriority from string
    fn priority_from_string(s: &str) -> MessagePriority {
        match s {
            "High" => MessagePriority::High,
            "Normal" => MessagePriority::Normal,
            "Low" => MessagePriority::Low,
            _ => MessagePriority::Normal,
        }
    }

    /// Convert MessageStatus to string
    fn status_to_string(status: MessageStatus) -> String {
        match status {
            MessageStatus::Pending => "Pending".to_string(),
            MessageStatus::Processing => "Processing".to_string(),
            MessageStatus::Completed => "Completed".to_string(),
            MessageStatus::Failed => "Failed".to_string(),
        }
    }

    /// Parse MessageStatus from string
    fn status_from_string(s: &str) -> MessageStatus {
        match s {
            "Pending" => MessageStatus::Pending,
            "Processing" => MessageStatus::Processing,
            "Completed" => MessageStatus::Completed,
            "Failed" => MessageStatus::Failed,
            _ => MessageStatus::Pending,
        }
    }

    /// Convert MessageRecord to StoredMessage
    fn record_to_stored(record: &MessageRecord) -> StoredMessage {
        StoredMessage {
            id: record.id.to_string(),
            from: record.from.clone(),
            payload: record.payload.clone(),
            priority: Self::priority_to_string(record.priority),
            priority_num: Self::priority_to_num(record.priority),
            status: Self::status_to_string(record.status),
            created_at: record.created_at as f64,
        }
    }

    /// Convert StoredMessage to MessageRecord
    fn stored_to_record(stored: StoredMessage) -> Result<MessageRecord> {
        let id = Uuid::parse_str(&stored.id)
            .map_err(|e| MailboxError::DeserializationError(format!("Invalid UUID: {}", e)))?;

        Ok(MessageRecord {
            id,
            from: stored.from,
            payload: stored.payload,
            priority: Self::priority_from_string(&stored.priority),
            status: Self::status_from_string(&stored.status),
            created_at: stored.created_at as u64,
        })
    }
}

#[async_trait::async_trait(?Send)]
impl Mailbox for IndexedDbMailbox {
    async fn enqueue(
        &self,
        from: Vec<u8>,
        payload: Vec<u8>,
        priority: MessagePriority,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let created_at = Self::now_ms();

        let record = MessageRecord {
            id,
            from,
            payload,
            priority,
            status: MessageStatus::Pending,
            created_at,
        };

        let stored = Self::record_to_stored(&record);

        // Start transaction
        let tx = self
            .rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to create tx: {:?}", e)))?;

        let store = tx
            .store(STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to get store: {:?}", e)))?;

        // Serialize and add
        let js_value = serde_wasm_bindgen::to_value(&stored)
            .map_err(|e| MailboxError::SerializationError(format!("{:?}", e)))?;

        store
            .add(&js_value, None)
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to add: {:?}", e)))?;

        // Commit transaction
        tx.done()
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Transaction failed: {:?}", e)))?;

        console::log_1(&format!("Enqueued message: {} (priority: {:?})", id, priority).into());

        Ok(id)
    }

    async fn dequeue(&self, limit: usize) -> Result<Vec<MessageRecord>> {
        // Start transaction
        let tx = self
            .rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to create tx: {:?}", e)))?;

        let store = tx
            .store(STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to get store: {:?}", e)))?;

        // Get all records (rexie doesn't support cursor-based iteration yet,
        // so we'll get all and filter/sort in memory for now)
        let all_values = store
            .get_all(None, None)
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to get all: {:?}", e)))?;

        let mut messages = Vec::new();

        for js_value in all_values {
            let stored: StoredMessage = serde_wasm_bindgen::from_value(js_value)
                .map_err(|e| MailboxError::DeserializationError(format!("{:?}", e)))?;

            let record = Self::stored_to_record(stored)?;

            // Only return pending messages
            if record.status == MessageStatus::Pending {
                messages.push(record);
            }
        }

        // Sort by priority (high first) then by created_at (oldest first)
        messages.sort_by(|a, b| {
            use std::cmp::Ordering;
            match b.priority.cmp(&a.priority) {
                Ordering::Equal => a.created_at.cmp(&b.created_at),
                other => other,
            }
        });

        // Take up to limit
        messages.truncate(limit);

        console::log_1(&format!("Dequeued {} messages", messages.len()).into());

        Ok(messages)
    }

    async fn ack(&self, message_id: Uuid) -> Result<()> {
        // Start transaction
        let tx = self
            .rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to create tx: {:?}", e)))?;

        let store = tx
            .store(STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to get store: {:?}", e)))?;

        // Delete the message
        let key = JsValue::from_str(&message_id.to_string());
        store
            .delete(key)
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to delete: {:?}", e)))?;

        // Commit transaction
        tx.done()
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Transaction failed: {:?}", e)))?;

        console::log_1(&format!("Acknowledged message: {}", message_id).into());

        Ok(())
    }

    async fn stats(&self) -> Result<MailboxStats> {
        // Start transaction
        let tx = self
            .rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to create tx: {:?}", e)))?;

        let store = tx
            .store(STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to get store: {:?}", e)))?;

        // Get all records
        let all_values = store
            .get_all(None, None)
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to get all: {:?}", e)))?;

        let mut stats = MailboxStats {
            total_messages: all_values.len(),
            pending_messages: 0,
            processing_messages: 0,
            high_priority_count: 0,
            normal_priority_count: 0,
            low_priority_count: 0,
        };

        for js_value in all_values {
            if let Ok(stored) = serde_wasm_bindgen::from_value::<StoredMessage>(js_value) {
                if let Ok(record) = Self::stored_to_record(stored) {
                    match record.status {
                        MessageStatus::Pending => stats.pending_messages += 1,
                        MessageStatus::Processing => stats.processing_messages += 1,
                        MessageStatus::Completed | MessageStatus::Failed => {}
                    }

                    match record.priority {
                        MessagePriority::High => stats.high_priority_count += 1,
                        MessagePriority::Normal => stats.normal_priority_count += 1,
                        MessagePriority::Low => stats.low_priority_count += 1,
                    }
                }
            }
        }

        console::log_1(&format!("Mailbox stats: {:?}", stats).into());

        Ok(stats)
    }

    async fn clear(&self) -> Result<()> {
        // Start transaction
        let tx = self
            .rexie
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to create tx: {:?}", e)))?;

        let store = tx
            .store(STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to get store: {:?}", e)))?;

        // Clear all records
        store
            .clear()
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to clear: {:?}", e)))?;

        // Commit transaction
        tx.done()
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Transaction failed: {:?}", e)))?;

        console::log_1(&"Mailbox cleared".into());

        Ok(())
    }
}
