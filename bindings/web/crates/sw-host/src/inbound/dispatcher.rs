//! Inbound Packet Dispatcher
//!
//! Routes incoming messages to the correct handling path based on PayloadType:
//! - `RPC_RELIABLE` / `RPC_SIGNAL` -> Mailbox (State Path)
//!
//! `STREAM_*` and `MEDIA_RTP` are owned by the DOM-side JS Fast Path and are
//! never delivered to this dispatcher. The historical Rust-side DOM lane was
//! retired when the SW↔DOM control plane moved into JS (see TD-001 in
//! `tech-debt.zh.md`).

use actr_mailbox_web::{Mailbox, MessagePriority};
use actr_web_common::{MessageFormat, PayloadType, WebError, WebResult};
use std::sync::Arc;

use crate::inbound::MailboxNotifier;

/// Inbound message dispatcher.
///
/// Web counterpart of actr's `InboundPacketDispatcher`.
pub struct InboundPacketDispatcher {
    /// Mailbox for State Path RPC messages.
    mailbox: Arc<dyn Mailbox>,

    /// Notifier used to wake MailboxProcessor when new messages arrive.
    notifier: Option<MailboxNotifier>,
}

impl InboundPacketDispatcher {
    /// Create a new dispatcher.
    pub fn new(mailbox: Arc<dyn Mailbox>) -> Self {
        Self {
            mailbox,
            notifier: None,
        }
    }

    /// Attach a MailboxNotifier so enqueue operations can wake MailboxProcessor.
    pub fn with_notifier(mut self, notifier: MailboxNotifier) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Dispatch a received message.
    ///
    /// # Parameters
    /// - `from`: Serialized sender ID bytes
    /// - `message`: Parsed message containing PayloadType and payload
    pub async fn dispatch(&self, from: Vec<u8>, message: MessageFormat) -> WebResult<()> {
        match message.payload_type {
            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                // State Path: enqueue into the Mailbox.
                self.dispatch_to_mailbox(from, message).await
            }
            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                // Fast Path is handled in the DOM-side JS layer and must never arrive here.
                log::error!(
                    "[InboundPacketDispatcher] Unexpected STREAM payload routed to Rust dispatcher; \
                     fast-path traffic should stay in the DOM-side JS bridge"
                );
                Err(WebError::Protocol(
                    "STREAM payload reached SW Rust dispatcher".to_string(),
                ))
            }
            PayloadType::MediaRtp => {
                // MediaRtp is delivered via WebRTC track primitives in the DOM-side JS layer.
                log::error!(
                    "[InboundPacketDispatcher] Unexpected MEDIA_RTP routed to Rust dispatcher; \
                     media frames should arrive via RTCTrackRemote on the DOM side"
                );
                Err(WebError::Protocol(
                    "MEDIA_RTP reached SW Rust dispatcher".to_string(),
                ))
            }
        }
    }

    /// Dispatch to the Mailbox through the State Path.
    async fn dispatch_to_mailbox(&self, from: Vec<u8>, message: MessageFormat) -> WebResult<()> {
        let priority = match message.payload_type {
            PayloadType::RpcSignal => MessagePriority::High,
            PayloadType::RpcReliable => MessagePriority::Normal,
            _ => {
                return Err(WebError::Protocol(format!(
                    "Invalid PayloadType for Mailbox: {:?}",
                    message.payload_type
                )));
            }
        };

        let message_id = self
            .mailbox
            .enqueue(from.clone(), message.data.to_vec(), priority)
            .await
            .map_err(|e| WebError::Mailbox(format!("Enqueue failed: {}", e)))?;

        log::debug!(
            "[InboundPacketDispatcher] Message enqueued to Mailbox: id={}, priority={:?}",
            message_id,
            priority
        );

        // Notify MailboxProcessor that new work is available.
        if let Some(ref notifier) = self.notifier {
            notifier.notify();
        }

        Ok(())
    }

}

#[cfg(test)]
#[allow(clippy::arc_with_non_send_sync)]
mod tests {
    use super::*;
    use actr_mailbox_web::IndexedDbMailbox;
    use bytes::Bytes;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_dispatcher_creation() {
        let mailbox = Arc::new(
            IndexedDbMailbox::new()
                .await
                .expect("Failed to create mailbox"),
        );
        let _dispatcher = InboundPacketDispatcher::new(mailbox);
    }

    #[wasm_bindgen_test]
    async fn test_dispatch_rpc_to_mailbox() {
        let mailbox = Arc::new(
            IndexedDbMailbox::new()
                .await
                .expect("Failed to create mailbox"),
        );

        // Clear the mailbox.
        mailbox.clear().await.expect("Failed to clear mailbox");

        let dispatcher = InboundPacketDispatcher::new(mailbox.clone());

        let from = b"test-sender".to_vec();
        let data = Bytes::from("test message");
        let message = MessageFormat::new(PayloadType::RpcReliable, data);

        dispatcher
            .dispatch(from, message)
            .await
            .expect("Dispatch failed");

        // Verify the message entered the Mailbox.
        let stats = mailbox.stats().await.expect("Failed to get stats");
        assert!(stats.pending_messages > 0);
    }
}
