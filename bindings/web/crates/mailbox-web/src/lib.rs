//! IndexedDB-based Mailbox for Actor-RTC Web
//!
//! # 组件
//!
//! - **IndexedDbMailbox**: 主 Mailbox 实现，支持优先级队列
//! - **DeadLetterQueue**: 死信队列，存储处理失败的消息

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

mod dead_letter_queue;
mod indexeddb;

pub use dead_letter_queue::{DeadLetterQueue, DeadLetterRecord};
pub use indexeddb::IndexedDbMailbox;

/// Mailbox error type
#[derive(Error, Debug)]
pub enum MailboxError {
    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Database error: {0}")]
    DatabaseError(String),
}

pub type Result<T> = std::result::Result<T, MailboxError>;

/// Message priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MessagePriority {
    Low = 0,
    Normal = 1,
    High = 2,
}

/// Message record stored in the mailbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    /// Unique message ID
    pub id: Uuid,

    /// Sender address (serialized)
    pub from: Vec<u8>,

    /// Message payload
    pub payload: Vec<u8>,

    /// Message priority
    pub priority: MessagePriority,

    /// Creation timestamp (milliseconds since epoch)
    pub created_at: u64,

    /// Processing status
    pub status: MessageStatus,
}

/// Message processing status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

/// Mailbox statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxStats {
    pub total_messages: usize,
    pub pending_messages: usize,
    pub processing_messages: usize,
    pub high_priority_count: usize,
    pub normal_priority_count: usize,
    pub low_priority_count: usize,
}

/// Mailbox trait
#[async_trait::async_trait(?Send)]
pub trait Mailbox {
    /// Enqueue a message
    async fn enqueue(
        &self,
        from: Vec<u8>,
        payload: Vec<u8>,
        priority: MessagePriority,
    ) -> Result<Uuid>;

    /// Dequeue messages (up to limit)
    async fn dequeue(&self, limit: usize) -> Result<Vec<MessageRecord>>;

    /// Acknowledge message completion
    async fn ack(&self, message_id: Uuid) -> Result<()>;

    /// Get mailbox statistics
    async fn stats(&self) -> Result<MailboxStats>;

    /// Clear all messages
    async fn clear(&self) -> Result<()>;
}
