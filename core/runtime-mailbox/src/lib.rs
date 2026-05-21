//! # Actor-RTC Mailbox Layer
//!
//! Persistent mailbox layer for the Actor-RTC framework, backed by SQLite.
//!
//! ## Core Features
//!
//! - **Message Persistence**: Reliable message queue and mailbox storage
//! - **Dead Letter Queue**: Poison message isolation and manual intervention
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use actr_runtime_mailbox::prelude::*;
//! use actr_protocol::{ActrId, Realm, ActrType};
//! use actr_protocol::prost::Message as ProstMessage;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create SQLite mailbox storage
//!     let mailbox = SqliteMailbox::new("./data/mailbox.db").await?;
//!
//!     // Create sender ActrId and serialize
//!     let sender = ActrId {
//!         realm: Realm { realm_id: 1 },
//!         serial_number: 1000,
//!         r#type: ActrType {
//!             manufacturer: "example".to_string(),
//!             name: "TestActor".to_string(),
//!             version: "1.0.0".to_string(),
//!         },
//!     };
//!     let mut from_bytes = Vec::new();
//!     sender.encode(&mut from_bytes)?;
//!
//!     let message = b"Hello, World!".to_vec();
//!
//!     // Enqueue message (from is the sender's ActrId as Protobuf bytes)
//!     let message_id = mailbox.enqueue(from_bytes, message, MessagePriority::Normal).await?;
//!
//!     // Dequeue messages
//!     let messages = mailbox.dequeue().await?;
//!     println!("Retrieved {} messages", messages.len());
//!
//!     // Acknowledge message
//!     if let Some(msg) = messages.first() {
//!         mailbox.ack(msg.id).await?;
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod mailbox;
#[cfg(feature = "sqlite")]
pub mod sqlite;

// Dead Letter Queue modules
pub mod dlq;
#[cfg(feature = "sqlite")]
pub mod sqlite_dlq;

// Re-export core types
pub use actr_protocol::{ActrError, ActrId};

// Storage layer core interfaces
pub use error::{StorageError, StorageResult};
pub use mailbox::{
    Mailbox, MailboxDepthObserver, MailboxStats, MessagePriority, MessageRecord, MessageStatus,
};

// DLQ core interfaces
pub use dlq::{DeadLetterQueue, DlqQuery, DlqRecord, DlqStats};

// Backend implementations (only available with sqlite feature)
#[cfg(feature = "sqlite")]
pub use sqlite::{SqliteConfig, SqliteMailbox};
#[cfg(feature = "sqlite")]
pub use sqlite_dlq::SqliteDeadLetterQueue;

pub mod prelude {
    //! Convenience imports for commonly used mailbox layer types and traits

    pub use crate::error::{StorageError, StorageResult};
    pub use crate::mailbox::{
        Mailbox, MailboxDepthObserver, MailboxStats, MessagePriority, MessageRecord, MessageStatus,
    };
    #[cfg(feature = "sqlite")]
    pub use crate::sqlite::{SqliteConfig, SqliteMailbox};

    // Dead Letter Queue
    pub use crate::dlq::{DeadLetterQueue, DlqQuery, DlqRecord, DlqStats};
    #[cfg(feature = "sqlite")]
    pub use crate::sqlite_dlq::SqliteDeadLetterQueue;

    // Base types
    pub use actr_protocol::{ActrError, ActrId};

    // Async trait support
    pub use async_trait::async_trait;

    // Common utilities
    pub use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
    pub use chrono::{DateTime, Utc};
    pub use uuid::Uuid;
}
