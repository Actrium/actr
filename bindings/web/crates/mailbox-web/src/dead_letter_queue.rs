//! DeadLetterQueue
//!
//! Stores messages that failed processing for later analysis or retry.

use crate::{MailboxError, MessageRecord, Result};
use rexie::{ObjectStore, Rexie, TransactionMode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const DLQ_DB_NAME: &str = "actr_dead_letter_queue";
const DLQ_STORE_NAME: &str = "dead_letters";
const DLQ_DB_VERSION: u32 = 1;

/// Dead letter record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterRecord {
    /// Original message.
    pub message: MessageRecord,

    /// Failure reason.
    pub reason: String,

    /// Failure timestamp.
    pub failed_at: u64,

    /// Retry count.
    pub retry_count: u32,

    /// Time of the last retry.
    pub last_retry_at: Option<u64>,
}

/// Dead letter queue.
pub struct DeadLetterQueue {
    db: Rexie,
}

impl DeadLetterQueue {
    /// Create a new dead letter queue.
    pub async fn new() -> Result<Self> {
        let db = Rexie::builder(DLQ_DB_NAME)
            .version(DLQ_DB_VERSION)
            .add_object_store(ObjectStore::new(DLQ_STORE_NAME).key_path("message.id"))
            .build()
            .await
            .map_err(|e| MailboxError::DatabaseError(format!("Failed to create DLQ: {}", e)))?;

        log::info!("[DeadLetterQueue] Created successfully");
        Ok(Self { db })
    }

    /// Add a dead letter.
    pub async fn add(&self, message: MessageRecord, reason: String) -> Result<()> {
        let record = DeadLetterRecord {
            message,
            reason,
            failed_at: js_sys::Date::now() as u64,
            retry_count: 0,
            last_retry_at: None,
        };

        let transaction = self
            .db
            .transaction(&[DLQ_STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let store = transaction
            .store(DLQ_STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let js_value = serde_wasm_bindgen::to_value(&record)
            .map_err(|e| MailboxError::SerializationError(e.to_string()))?;

        store
            .put(&js_value, None)
            .await
            .map_err(|e| MailboxError::StorageError(e.to_string()))?;

        transaction
            .done()
            .await
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        log::info!(
            "[DeadLetterQueue] Added dead letter: id={}, reason={}",
            record.message.id,
            record.reason
        );

        Ok(())
    }

    /// Get all dead letters.
    pub async fn get_all(&self) -> Result<Vec<DeadLetterRecord>> {
        let transaction = self
            .db
            .transaction(&[DLQ_STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let store = transaction
            .store(DLQ_STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let js_records = store
            .get_all(None, None)
            .await
            .map_err(|e| MailboxError::StorageError(e.to_string()))?;

        let mut records = Vec::new();
        for js_value in js_records {
            if let Ok(record) = serde_wasm_bindgen::from_value::<DeadLetterRecord>(js_value) {
                records.push(record);
            }
        }

        Ok(records)
    }

    /// Retry a dead letter.
    ///
    /// Returns the message record for reprocessing.
    pub async fn retry(&self, message_id: Uuid) -> Result<MessageRecord> {
        let transaction = self
            .db
            .transaction(&[DLQ_STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let store = transaction
            .store(DLQ_STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let key = serde_wasm_bindgen::to_value(&message_id.to_string())
            .map_err(|e| MailboxError::SerializationError(e.to_string()))?;

        let js_value = store
            .get(key)
            .await
            .map_err(|e| MailboxError::StorageError(e.to_string()))?;

        let js_value = js_value.ok_or_else(|| {
            MailboxError::NotFound(format!("Dead letter not found: {}", message_id))
        })?;

        let mut record: DeadLetterRecord = serde_wasm_bindgen::from_value(js_value)
            .map_err(|e| MailboxError::DeserializationError(e.to_string()))?;

        // Update retry metadata.
        record.retry_count += 1;
        record.last_retry_at = Some(js_sys::Date::now() as u64);

        let js_value = serde_wasm_bindgen::to_value(&record)
            .map_err(|e| MailboxError::SerializationError(e.to_string()))?;

        store
            .put(&js_value, None)
            .await
            .map_err(|e| MailboxError::StorageError(e.to_string()))?;

        transaction
            .done()
            .await
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        log::info!(
            "[DeadLetterQueue] Retrying dead letter: id={}, retry_count={}",
            message_id,
            record.retry_count
        );

        Ok(record.message)
    }

    /// Remove a dead letter.
    pub async fn remove(&self, message_id: Uuid) -> Result<()> {
        let transaction = self
            .db
            .transaction(&[DLQ_STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let store = transaction
            .store(DLQ_STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let key = serde_wasm_bindgen::to_value(&message_id.to_string())
            .map_err(|e| MailboxError::SerializationError(e.to_string()))?;

        store
            .delete(key)
            .await
            .map_err(|e| MailboxError::StorageError(e.to_string()))?;

        transaction
            .done()
            .await
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        log::info!("[DeadLetterQueue] Removed dead letter: id={}", message_id);

        Ok(())
    }

    /// Clear all dead letters.
    pub async fn clear(&self) -> Result<()> {
        let transaction = self
            .db
            .transaction(&[DLQ_STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let store = transaction
            .store(DLQ_STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        store
            .clear()
            .await
            .map_err(|e| MailboxError::StorageError(e.to_string()))?;

        transaction
            .done()
            .await
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        log::info!("[DeadLetterQueue] Cleared all dead letters");

        Ok(())
    }

    /// Get the number of dead letters.
    pub async fn count(&self) -> Result<usize> {
        let transaction = self
            .db
            .transaction(&[DLQ_STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let store = transaction
            .store(DLQ_STORE_NAME)
            .map_err(|e| MailboxError::DatabaseError(e.to_string()))?;

        let count = store
            .count(None)
            .await
            .map_err(|e| MailboxError::StorageError(e.to_string()))?;

        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    // Tests must run in a browser environment.
}
