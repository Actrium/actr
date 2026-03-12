//! # Actor Mailbox
//!
//! This module defines the core interfaces and data structures for persistent message queues.
//!
//! ## Reliable Queue Workflow
//!
//! This interface is designed as a reliable queue to prevent message loss when a consumer
//! crashes during message processing. The workflow is as follows:
//!
//! 1.  **`dequeue()`**: The consumer retrieves a batch of messages from the queue. These messages
//!     are atomically marked as `Inflight` in the database, but are **not deleted**.
//! 2.  **Process messages**: The consumer processes these messages locally.
//! 3.  **`ack()`**: When a message has been successfully processed, the consumer calls
//!     `ack(message_id)`. This **permanently deletes** the message, marking the successful
//!     completion of the work unit.
//!
//! If the consumer crashes after `dequeue` but before `ack`, those `Inflight` messages remain
//! in the database. On the next consumer restart, a "cleanup" routine can be implemented to
//! reprocess these "stuck" messages.

use crate::error::StorageResult;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Message priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MessagePriority {
    Normal,
    High,
}

/// Message record retrieved from the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    /// Message ID
    pub id: Uuid,
    /// Sender's ActrId (Protobuf bytes)
    ///
    /// # Design notes
    /// - `from` stores raw Protobuf bytes without deserializing into ActrId struct
    /// - Avoids the decode -> ActrId -> encode round-trip
    /// - Only deserialize when actually needed
    /// - Gateway passes bytes directly, zero overhead
    /// - All messages entering the Mailbox originate from WebRTC and always have a sender
    pub from: Vec<u8>,
    /// Message content (raw bytes, not unpacked)
    pub payload: Vec<u8>,
    /// Priority
    pub priority: MessagePriority,
    /// Creation time
    pub created_at: DateTime<Utc>,
    /// Processing status
    pub status: MessageStatus,
}

/// Message processing status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageStatus {
    Queued,
    Inflight,
}

/// Mailbox statistics
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxStats {
    /// Total number of messages waiting in the queue
    pub queued_messages: u64,
    /// Total number of dequeued but not yet acknowledged messages
    pub inflight_messages: u64,
    /// Queued message count by priority
    pub queued_by_priority: std::collections::HashMap<MessagePriority, u64>,
}

/// Mailbox interface - defines core operations for message persistence
///
/// ## Usage example: `dequeue -> process -> ack` loop
///
/// The `dequeue` method automatically retrieves the next batch of messages. Callers need not
/// worry about batch size; that detail is handled internally by the implementation.
///
/// ```rust,no_run
/// use actr_runtime_mailbox::prelude::*;
/// use std::time::Duration;
///
/// async fn message_processor(mailbox: impl Mailbox) {
///     loop {
///         // 1. Retrieve the next batch of messages from the queue
///         match mailbox.dequeue().await {
///             Ok(messages) => {
///                 if messages.is_empty() {
///                     tokio::time::sleep(Duration::from_secs(1)).await;
///                     continue;
///                 }
///
///                 // 2. Process messages one by one
///                 for msg in messages {
///                     println!("Processing message: {}", msg.id);
///                     // ... execute your business logic here ...
///
///                     // 3. After successful processing, acknowledge this message
///                     if let Err(e) = mailbox.ack(msg.id).await {
///                         eprintln!("Failed to ack message {}: {}", msg.id, e);
///                     }
///                 }
///             }
///             Err(e) => {
///                 eprintln!("Failed to dequeue messages: {}", e);
///                 tokio::time::sleep(Duration::from_secs(5)).await; // Database error, wait longer
///             }
///         }
///     }
/// }
/// ```
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Mailbox: Send + Sync {
    /// Enqueue a message.
    ///
    /// # Arguments
    /// - `from`: Sender's ActrId (Protobuf bytes, provided directly by Gateway, not unpacked)
    /// - `payload`: Message content (raw bytes, not unpacked)
    /// - `priority`: Message priority
    async fn enqueue(
        &self,
        from: Vec<u8>,
        payload: Vec<u8>,
        priority: MessagePriority,
    ) -> StorageResult<Uuid>;

    /// Dequeue a batch of messages from the queue.
    ///
    /// This method automatically handles priority: as long as high-priority messages exist,
    /// they are returned first. Dequeued messages are atomically marked as `Inflight` but
    /// not deleted. You must call `ack()` after processing to permanently remove them.
    async fn dequeue(&self) -> StorageResult<Vec<MessageRecord>>;

    /// Acknowledge that a message has been successfully processed, permanently removing it from the queue.
    async fn ack(&self, message_id: Uuid) -> StorageResult<()>;

    /// Get current mailbox statistics.
    async fn status(&self) -> StorageResult<MailboxStats>;
}
