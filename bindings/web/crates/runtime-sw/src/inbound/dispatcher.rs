//! Inbound Packet Dispatcher
//!
//! Routes incoming messages to the correct handling path based on PayloadType:
//! - `RPC_RELIABLE` / `RPC_SIGNAL` -> Mailbox (State Path)
//! - `STREAM_RELIABLE` / `STREAM_LATENCY_FIRST` -> DOM StreamHandlerRegistry (Fast Path)
//! - `MEDIA_RTP` -> DOM MediaFrameRegistry (Fast Path, typically via WebRTC MediaTrack)

use actr_mailbox_web::{Mailbox, MessagePriority};
use actr_web_common::{MessageFormat, PayloadType, WebError, WebResult};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::inbound::MailboxNotifier;
use crate::transport::DataLane;

/// Inbound message dispatcher.
///
/// Web counterpart of actr's `InboundPacketDispatcher`.
pub struct InboundPacketDispatcher {
    /// Mailbox for State Path RPC messages.
    mailbox: Arc<dyn Mailbox>,

    /// DOM communication lane used to forward Fast Path traffic.
    dom_lane: Arc<Mutex<Option<DataLane>>>,

    /// Notifier used to wake MailboxProcessor when new messages arrive.
    notifier: Option<MailboxNotifier>,
}

impl InboundPacketDispatcher {
    /// Create a new dispatcher.
    pub fn new(mailbox: Arc<dyn Mailbox>) -> Self {
        Self {
            mailbox,
            dom_lane: Arc::new(Mutex::new(None)),
            notifier: None,
        }
    }

    /// Attach a MailboxNotifier so enqueue operations can wake MailboxProcessor.
    pub fn with_notifier(mut self, notifier: MailboxNotifier) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Set the DOM communication lane.
    pub fn set_dom_lane(&self, lane: DataLane) {
        let mut dom_lane = self.dom_lane.lock();
        *dom_lane = Some(lane);
        log::info!("[InboundPacketDispatcher] DOM lane set");
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
                // Fast Path: forward to the DOM StreamHandlerRegistry.
                self.dispatch_to_stream_registry(from, message).await
            }
            PayloadType::MediaRtp => {
                // Fast Path: forward to the DOM MediaFrameRegistry.
                // MediaRtp normally should not arrive through DataChannel and is expected
                // to flow through WebRTC track primitives instead.
                log::warn!(
                    "[InboundPacketDispatcher] Received MEDIA_RTP via DataChannel, \
                     this is unusual. Media should come via RTCTrackRemote."
                );
                self.dispatch_to_media_registry(from, message).await
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

    /// Dispatch to StreamHandlerRegistry through the Fast Path.
    ///
    /// Forwards the message to the DOM-side StreamHandlerRegistry through the DOM lane.
    async fn dispatch_to_stream_registry(
        &self,
        _from: Vec<u8>,
        message: MessageFormat,
    ) -> WebResult<()> {
        let lane_opt = {
            let dom_lane = self.dom_lane.lock();
            dom_lane.clone()
        };

        if let Some(ref lane) = lane_opt {
            // Forward to the DOM, which will parse and dispatch into StreamHandlerRegistry.
            lane.send(message.data.clone())
                .await
                .map_err(|e| WebError::Transport(format!("Failed to forward to DOM: {}", e)))?;

            log::debug!("[InboundPacketDispatcher] Stream message forwarded to DOM");
        } else {
            log::warn!("[InboundPacketDispatcher] DOM lane not set, cannot forward stream message");
        }

        Ok(())
    }

    /// Dispatch to MediaFrameRegistry through the Fast Path.
    ///
    /// Forwards the message to the DOM-side MediaFrameRegistry through the DOM lane.
    async fn dispatch_to_media_registry(
        &self,
        _from: Vec<u8>,
        message: MessageFormat,
    ) -> WebResult<()> {
        let lane_opt = {
            let dom_lane = self.dom_lane.lock();
            dom_lane.clone()
        };

        if let Some(ref lane) = lane_opt {
            // Forward to the DOM, which will parse and dispatch into MediaFrameRegistry.
            lane.send(message.data.clone())
                .await
                .map_err(|e| WebError::Transport(format!("Failed to forward to DOM: {}", e)))?;

            log::debug!("[InboundPacketDispatcher] Media frame forwarded to DOM");
        } else {
            log::warn!("[InboundPacketDispatcher] DOM lane not set, cannot forward media frame");
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
