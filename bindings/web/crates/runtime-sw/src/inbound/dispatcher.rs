//! Inbound Packet Dispatcher
//!
//! 根据 PayloadType 将接收到的消息路由到正确的处理路径：
//! - RPC_RELIABLE/RPC_SIGNAL → Mailbox (State Path)
//! - STREAM_RELIABLE/STREAM_LATENCY_FIRST → 转发到 DOM 的 StreamHandlerRegistry (Fast Path)
//! - MEDIA_RTP → 转发到 DOM 的 MediaFrameRegistry (Fast Path，通过 WebRTC MediaTrack)

use actr_mailbox_web::{Mailbox, MessagePriority};
use actr_web_common::{MessageFormat, PayloadType, WebError, WebResult};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::inbound::MailboxNotifier;
use crate::transport::DataLane;

/// 入站消息分发器
///
/// 对标 actr 的 InboundPacketDispatcher
pub struct InboundPacketDispatcher {
    /// Mailbox 用于 State Path（RPC 消息）
    mailbox: Arc<dyn Mailbox>,

    /// DOM 通信通道（用于转发 Fast Path 消息）
    dom_lane: Arc<Mutex<Option<DataLane>>>,

    /// 通知 MailboxProcessor 有新消息可处理
    notifier: Option<MailboxNotifier>,
}

impl InboundPacketDispatcher {
    /// 创建新的分发器
    pub fn new(mailbox: Arc<dyn Mailbox>) -> Self {
        Self {
            mailbox,
            dom_lane: Arc::new(Mutex::new(None)),
            notifier: None,
        }
    }

    /// 设置 MailboxNotifier，使得入队后能唤醒 MailboxProcessor
    pub fn with_notifier(mut self, notifier: MailboxNotifier) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// 设置 DOM 通信通道
    pub fn set_dom_lane(&self, lane: DataLane) {
        let mut dom_lane = self.dom_lane.lock();
        *dom_lane = Some(lane);
        log::info!("[InboundPacketDispatcher] DOM lane set");
    }

    /// 分发接收到的消息
    ///
    /// # 参数
    /// - `from`: 发送方 ID（序列化的 bytes）
    /// - `message`: 消息格式（包含 PayloadType 和数据）
    pub async fn dispatch(&self, from: Vec<u8>, message: MessageFormat) -> WebResult<()> {
        match message.payload_type {
            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                // State Path: 进入 Mailbox
                self.dispatch_to_mailbox(from, message).await
            }
            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                // Fast Path: 转发到 DOM 的 StreamHandlerRegistry
                self.dispatch_to_stream_registry(from, message).await
            }
            PayloadType::MediaRtp => {
                // Fast Path: 转发到 DOM 的 MediaFrameRegistry
                // 注意：MediaRtp 通常不应该通过 DataChannel 接收
                // 应该通过 WebRTC 的 RTCTrackRemote
                log::warn!(
                    "[InboundPacketDispatcher] Received MEDIA_RTP via DataChannel, \
                     this is unusual. Media should come via RTCTrackRemote."
                );
                self.dispatch_to_media_registry(from, message).await
            }
        }
    }

    /// 分发到 Mailbox（State Path）
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

        // 通知 MailboxProcessor 有新消息可处理（事件驱动）
        if let Some(ref notifier) = self.notifier {
            notifier.notify();
        }

        Ok(())
    }

    /// 分发到 StreamHandlerRegistry（Fast Path）
    ///
    /// 通过 DOM lane 转发消息到 DOM 侧的 StreamHandlerRegistry
    async fn dispatch_to_stream_registry(
        &self,
        _from: Vec<u8>,
        message: MessageFormat,
    ) -> WebResult<()> {
        let dom_lane = self.dom_lane.lock();

        if let Some(ref lane) = *dom_lane {
            // 转发到 DOM（DOM 侧会解析并派发到 StreamHandlerRegistry）
            lane.send(message.data.clone())
                .await
                .map_err(|e| WebError::Transport(format!("Failed to forward to DOM: {}", e)))?;

            log::debug!("[InboundPacketDispatcher] Stream message forwarded to DOM");
        } else {
            log::warn!("[InboundPacketDispatcher] DOM lane not set, cannot forward stream message");
        }

        Ok(())
    }

    /// 分发到 MediaFrameRegistry（Fast Path）
    ///
    /// 通过 DOM lane 转发消息到 DOM 侧的 MediaFrameRegistry
    async fn dispatch_to_media_registry(
        &self,
        _from: Vec<u8>,
        message: MessageFormat,
    ) -> WebResult<()> {
        let dom_lane = self.dom_lane.lock();

        if let Some(ref lane) = *dom_lane {
            // 转发到 DOM（DOM 侧会解析并派发到 MediaFrameRegistry）
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

        // 清空 mailbox
        mailbox.clear().await.expect("Failed to clear mailbox");

        let dispatcher = InboundPacketDispatcher::new(mailbox.clone());

        let from = b"test-sender".to_vec();
        let data = Bytes::from("test message");
        let message = MessageFormat::new(PayloadType::RpcReliable, data);

        dispatcher
            .dispatch(from, message)
            .await
            .expect("Dispatch failed");

        // 验证消息进入了 Mailbox
        let stats = mailbox.stats().await.expect("Failed to get stats");
        assert!(stats.pending_messages > 0);
    }
}
