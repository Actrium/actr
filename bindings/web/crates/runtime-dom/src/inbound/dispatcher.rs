//! DOM 侧入站消息分发器
//!
//! 接收来自 SW 的 Fast Path 消息（STREAM_*/MEDIA_RTP）
//! 并派发到对应的注册器

use actr_web_common::{MessageFormat, PayloadType, WebError, WebResult};
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::fastpath::{MediaFrameHandlerRegistry, StreamHandlerRegistry};
use crate::transport::DataLane;

/// DOM 侧入站消息分发器
pub struct DomInboundDispatcher {
    /// Stream 处理器注册表
    stream_registry: Arc<StreamHandlerRegistry>,

    /// Media 处理器注册表
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// SW 通信通道（用于响应）
    sw_lane: Arc<Mutex<Option<DataLane>>>,
}

impl DomInboundDispatcher {
    /// 创建新的分发器
    pub fn new(
        stream_registry: Arc<StreamHandlerRegistry>,
        media_registry: Arc<MediaFrameHandlerRegistry>,
    ) -> Self {
        Self {
            stream_registry,
            media_registry,
            sw_lane: Arc::new(Mutex::new(None)),
        }
    }

    /// 设置 SW 通信通道
    pub fn set_sw_lane(&self, lane: DataLane) {
        let mut sw_lane = self.sw_lane.lock();
        *sw_lane = Some(lane);
        log::info!("[DomInboundDispatcher] SW lane set");
    }

    /// 分发接收到的消息
    ///
    /// # 参数
    /// - `data`: 原始消息数据
    pub fn dispatch(&self, data: Bytes) -> WebResult<()> {
        // 解析 MessageFormat
        let message = MessageFormat::try_from(data)?;

        match message.payload_type {
            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                self.dispatch_to_stream_registry(message)
            }
            PayloadType::MediaRtp => self.dispatch_to_media_registry(message),
            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                // RPC 消息不应该到 DOM，应该在 SW 处理
                log::warn!(
                    "[DomInboundDispatcher] Received RPC message in DOM, \
                     this should be handled in SW"
                );
                Err(WebError::Protocol(
                    "RPC messages should not arrive at DOM".to_string(),
                ))
            }
        }
    }

    /// 分发到 StreamHandlerRegistry
    fn dispatch_to_stream_registry(&self, message: MessageFormat) -> WebResult<()> {
        // 解析 stream_id（这里简化处理，实际可能需要更复杂的协议）
        // 假设数据格式：[stream_id_len(4) | stream_id(N) | chunk_data(M)]
        let data = message.data;
        if data.len() < 4 {
            return Err(WebError::Protocol(
                "Invalid stream message format".to_string(),
            ));
        }

        let stream_id_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + stream_id_len {
            return Err(WebError::Protocol(
                "Invalid stream message format".to_string(),
            ));
        }

        let stream_id = String::from_utf8(data[4..4 + stream_id_len].to_vec())
            .map_err(|e| WebError::Protocol(format!("Invalid stream_id: {}", e)))?;

        let chunk_data = data.slice(4 + stream_id_len..);

        // 派发到注册器
        self.stream_registry.dispatch(&stream_id, chunk_data);

        log::debug!(
            "[DomInboundDispatcher] Stream message dispatched: stream_id={}",
            stream_id
        );

        Ok(())
    }

    /// 分发到 MediaFrameRegistry
    fn dispatch_to_media_registry(&self, message: MessageFormat) -> WebResult<()> {
        // 解析 track_id（这里简化处理，实际可能需要更复杂的协议）
        // 假设数据格式：[track_id_len(4) | track_id(N) | frame_data(M)]
        let data = message.data;
        if data.len() < 4 {
            return Err(WebError::Protocol(
                "Invalid media message format".to_string(),
            ));
        }

        let track_id_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + track_id_len {
            return Err(WebError::Protocol(
                "Invalid media message format".to_string(),
            ));
        }

        let track_id = String::from_utf8(data[4..4 + track_id_len].to_vec())
            .map_err(|e| WebError::Protocol(format!("Invalid track_id: {}", e)))?;

        let frame_data = data.slice(4 + track_id_len..);

        // 派发到注册器
        self.media_registry.dispatch(&track_id, frame_data);

        log::debug!(
            "[DomInboundDispatcher] Media frame dispatched: track_id={}",
            track_id
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_dispatcher_creation() {
        let stream_registry = Arc::new(StreamHandlerRegistry::new());
        let media_registry = Arc::new(MediaFrameHandlerRegistry::new());
        let _dispatcher = DomInboundDispatcher::new(stream_registry, media_registry);
    }

    #[wasm_bindgen_test]
    fn test_dispatch_stream_message() {
        let stream_registry = Arc::new(StreamHandlerRegistry::new());
        let media_registry = Arc::new(MediaFrameHandlerRegistry::new());
        let dispatcher = DomInboundDispatcher::new(stream_registry.clone(), media_registry);

        // 注册一个测试handler
        let received = Arc::new(parking_lot::Mutex::new(false));
        let received_clone = received.clone();
        stream_registry.register(
            "test-stream".to_string(),
            Arc::new(move |_data| {
                *received_clone.lock() = true;
            }),
        );

        // 构造测试消息
        // [stream_id_len(4) | stream_id(11="test-stream") | chunk_data(10="test-chunk")]
        let stream_id = b"test-stream";
        let chunk_data = b"test-chunk";
        let mut data = Vec::new();
        data.extend_from_slice(&(stream_id.len() as u32).to_be_bytes());
        data.extend_from_slice(stream_id);
        data.extend_from_slice(chunk_data);

        let message = MessageFormat::new(PayloadType::StreamReliable, Bytes::from(data));

        dispatcher
            .dispatch(message.to_bytes())
            .expect("Dispatch failed");

        // 验证回调被调用
        assert!(*received.lock());
    }
}
