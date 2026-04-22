//! Inbound message handling for Service Worker
//!
//! Message receive and dispatch logic that mirrors actr's inbound module.
//!
//! # Architecture
//!
//! ```text
//! InboundPacketDispatcher → Mailbox → MailboxProcessor → Scheduler → Actor
//! ```

mod dispatcher;
mod mailbox_processor;
mod scheduler;

pub use dispatcher::InboundPacketDispatcher;
pub use mailbox_processor::{MailboxMessageHandler, MailboxNotifier, MailboxProcessor};
pub use scheduler::{ActorHandler, Scheduler};
