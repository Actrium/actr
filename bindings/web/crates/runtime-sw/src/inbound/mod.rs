//! Inbound message handling for Service Worker
//!
//! 消息接收和分发逻辑，对标 actr 的 inbound 模块
//!
//! # 架构
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
