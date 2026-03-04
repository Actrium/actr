//! WebRTC DataChannel 消息接收处理
//!
//! 处理 DOM 侧 WebRTC DataChannel 接收到的消息

use actr_web_common::{MessageFormat, PayloadType, WebError, WebResult};
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, RtcDataChannel};

use crate::fastpath::{MediaFrameHandlerRegistry, StreamHandlerRegistry};
use crate::transport::DataLane;

/// WebRTC DataChannel 消息接收处理器
///
/// 处理从 WebRTC DataChannel 接收到的消息：
/// - RPC 消息：转发到 SW Mailbox
/// - Stream 消息：本地派发到 StreamHandlerRegistry
/// - Media 消息：警告（应该通过 MediaTrack）
pub struct WebRtcDataChannelReceiver {
    /// Stream 处理器注册表
    stream_registry: Arc<StreamHandlerRegistry>,

    /// Media 处理器注册表（虽然 Media 应该通过 Track）
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// SW 通信通道（用于转发 RPC 消息）
    sw_lane: Arc<Mutex<Option<DataLane>>>,
}

impl WebRtcDataChannelReceiver {
    /// 创建新的接收处理器
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
        log::info!("[WebRtcDataChannelReceiver] SW lane set");
    }

    /// 绑定到 DataChannel
    ///
    /// 设置 onmessage 回调
    pub fn attach_to_datachannel(&self, datachannel: &RtcDataChannel) -> WebResult<()> {
        let stream_registry = self.stream_registry.clone();
        let media_registry = self.media_registry.clone();
        let sw_lane = self.sw_lane.clone();

        let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
            // 处理接收到的消息
            if let Ok(array_buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);
                let data = uint8_array.to_vec();

                // 解析 MessageFormat
                match MessageFormat::try_from(data.as_slice()) {
                    Ok(message) => {
                        // 根据 PayloadType 路由
                        match message.payload_type {
                            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                                // RPC 消息：转发到 SW
                                Self::forward_rpc_to_sw(&sw_lane, message);
                            }
                            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                                // Stream 消息：本地派发
                                if let Err(e) =
                                    Self::dispatch_stream_local(&stream_registry, message)
                                {
                                    log::error!(
                                        "[WebRtcDataChannelReceiver] Stream dispatch failed: {}",
                                        e
                                    );
                                }
                            }
                            PayloadType::MediaRtp => {
                                // Media 消息：警告（应该通过 MediaTrack）
                                log::warn!(
                                    "[WebRtcDataChannelReceiver] Received MEDIA_RTP via DataChannel, \
                                     should use MediaTrack instead"
                                );
                                if let Err(e) = Self::dispatch_media_local(&media_registry, message)
                                {
                                    log::error!(
                                        "[WebRtcDataChannelReceiver] Media dispatch failed: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "[WebRtcDataChannelReceiver] Failed to parse MessageFormat: {}",
                            e
                        );
                    }
                }
            }
        }) as Box<dyn FnMut(_)>);

        datachannel.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        log::info!("[WebRtcDataChannelReceiver] Attached to DataChannel");
        Ok(())
    }

    /// 转发 RPC 消息到 SW
    fn forward_rpc_to_sw(sw_lane: &Arc<Mutex<Option<DataLane>>>, message: MessageFormat) {
        let sw_lane_guard = sw_lane.lock();

        if let Some(ref lane) = *sw_lane_guard {
            // 封装为控制消息并发送到 SW
            // 格式：[MessageType(1) | From(序列化) | MessageFormat(序列化)]
            // 这里简化处理，直接发送 MessageFormat
            let data = message.to_bytes();

            wasm_bindgen_futures::spawn_local({
                let lane = lane.clone();
                async move {
                    if let Err(e) = lane.send(data).await {
                        log::error!(
                            "[WebRtcDataChannelReceiver] Failed to forward RPC to SW: {}",
                            e
                        );
                    } else {
                        log::debug!(
                            "[WebRtcDataChannelReceiver] RPC message forwarded to SW: {:?}",
                            message.payload_type
                        );
                    }
                }
            });
        } else {
            log::warn!("[WebRtcDataChannelReceiver] SW lane not set, cannot forward RPC");
        }
    }

    /// 本地派发 Stream 消息
    fn dispatch_stream_local(
        stream_registry: &Arc<StreamHandlerRegistry>,
        message: MessageFormat,
    ) -> WebResult<()> {
        // 解析 stream_id（格式：[stream_id_len(4) | stream_id(N) | chunk_data(M)]）
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
        stream_registry.dispatch(&stream_id, chunk_data);

        log::debug!(
            "[WebRtcDataChannelReceiver] Stream message dispatched locally: stream_id={}",
            stream_id
        );

        Ok(())
    }

    /// 本地派发 Media 消息
    fn dispatch_media_local(
        media_registry: &Arc<MediaFrameHandlerRegistry>,
        message: MessageFormat,
    ) -> WebResult<()> {
        // 解析 track_id（格式：[track_id_len(4) | track_id(N) | frame_data(M)]）
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
        media_registry.dispatch(&track_id, frame_data);

        log::debug!(
            "[WebRtcDataChannelReceiver] Media frame dispatched locally: track_id={}",
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
    fn test_receiver_creation() {
        let stream_registry = Arc::new(StreamHandlerRegistry::new());
        let media_registry = Arc::new(MediaFrameHandlerRegistry::new());
        let _receiver = WebRtcDataChannelReceiver::new(stream_registry, media_registry);
    }
}
