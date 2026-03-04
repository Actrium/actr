//! DataLane - DOM 环境的数据传输通道
//!
//! DOM 端包含：
//! - PostMessage Lane：与 Service Worker 通信
//! - WebRTC DataChannel Lane：P2P 数据传输
//! - WebRTC MediaTrack Lane：媒体流传输

use actr_web_common::zero_copy::{
    construct_message_header, construct_message_zero_copy, send_with_transfer, send_zero_copy,
    should_use_transfer,
};
use actr_web_common::{PayloadType, WebError, WebResult};
use bytes::Bytes;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{MessagePort, RtcDataChannel};

/// DOM 环境传输层操作结果
pub type LaneResult<T> = WebResult<T>;

/// DataLane - DOM 环境的数据传输通道
#[derive(Clone)]
pub enum DataLane {
    /// PostMessage Lane（与 Service Worker 通信）
    ///
    /// 支持：全部 PayloadType
    PostMessage {
        port: Arc<MessagePort>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },

    /// WebRTC DataChannel Lane
    ///
    /// 支持：RPC_*, STREAM_* (不支持 MEDIA_RTP)
    WebRtcDataChannel {
        data_channel: Arc<RtcDataChannel>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },

    /// WebRTC MediaTrack Lane
    ///
    /// 仅支持：MEDIA_RTP
    WebRtcMediaTrack {
        track_id: String,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },
}

impl DataLane {
    /// 发送消息（零拷贝优化）
    pub async fn send(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage {
                port, payload_type, ..
            } => {
                // ✅ 构造消息：使用零拷贝辅助函数
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // ✅ 零拷贝发送：创建 WASM 内存视图
                let js_view = send_zero_copy(&msg);

                port.post_message(&js_view.into()).map_err(|e| {
                    WebError::Transport(format!("PostMessage send failed: {:?}", e))
                })?;

                log::trace!(
                    "PostMessage Lane (DOM) 发送消息: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );

                Ok(())
            }

            DataLane::WebRtcDataChannel {
                data_channel,
                payload_type,
                ..
            } => {
                use web_sys::RtcDataChannelState;

                // 检查 DataChannel 状态
                if data_channel.ready_state() != RtcDataChannelState::Open {
                    return Err(WebError::Transport("WebRTC DataChannel 未打开".to_string()));
                }

                // ✅ 构造消息：使用零拷贝辅助函数
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // ✅ 直接发送 Bytes slice（DataChannel API 接受 &[u8]）
                data_channel.send_with_u8_array(&msg).map_err(|e| {
                    WebError::Transport(format!("DataChannel send failed: {:?}", e))
                })?;

                log::trace!(
                    "WebRTC DataChannel Lane 发送消息: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );

                Ok(())
            }

            DataLane::WebRtcMediaTrack { track_id, .. } => Err(WebError::Transport(format!(
                "MediaTrack Lane (track_id={}) 不支持直接 send",
                track_id
            ))),
        }
    }

    /// 使用 Transferable Objects 发送消息（仅 PostMessage）
    ///
    /// **Transferable Objects**：
    /// - 转移 ArrayBuffer 所有权而非拷贝
    /// - 适用于大数据传输（>10KB）
    /// - 仅支持 PostMessage Lane（WebSocket/DataChannel 不支持）
    ///
    /// # 参数
    /// - `data`: 要发送的数据
    ///
    /// # 返回
    /// - `Ok(())`: 发送成功
    /// - `Err`: 发送失败或不支持的 Lane 类型
    ///
    /// # 使用建议
    /// - 大数据（>10KB）使用此方法
    /// - 小数据（<10KB）使用普通 `send()` 方法
    /// - 或使用 `send_auto()` 自动选择
    pub async fn send_with_transfer(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage {
                port, payload_type, ..
            } => {
                // ✅ 构造消息：使用零拷贝辅助函数
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // ✅ 使用 Transferable Objects 发送
                let (js_view, transfer_list) = send_with_transfer(&msg);

                // 使用 wasm-bindgen 的底层 API 调用 postMessage(message, transferList)
                let post_message_fn =
                    js_sys::Reflect::get(port.as_ref(), &JsValue::from_str("postMessage"))
                        .map_err(|e| {
                            WebError::Transport(format!("Failed to get postMessage: {:?}", e))
                        })?;

                let result = js_sys::Reflect::apply(
                    post_message_fn.unchecked_ref(),
                    port.as_ref(),
                    &js_sys::Array::of2(&js_view.into(), &transfer_list),
                );

                match result {
                    Ok(_) => {
                        log::trace!(
                            "PostMessage Lane (DOM) 使用 transfer 发送消息: payload_type={:?}, size={} bytes",
                            payload_type,
                            data.len()
                        );
                        Ok(())
                    }
                    Err(e) => Err(WebError::Transport(format!(
                        "PostMessage with transfer failed: {:?}",
                        e
                    ))),
                }
            }

            DataLane::WebRtcDataChannel { .. } => {
                // DataChannel 不支持 Transferable Objects，回退到普通 send
                log::warn!("WebRTC DataChannel 不支持 Transferable Objects，回退到普通 send");
                self.send(data).await
            }

            DataLane::WebRtcMediaTrack { track_id, .. } => Err(WebError::Transport(format!(
                "MediaTrack Lane (track_id={}) 不支持 send_with_transfer",
                track_id
            ))),
        }
    }

    /// 自动选择发送方式（根据数据大小）
    ///
    /// **决策逻辑**：
    /// - PostMessage + 数据 >= 10KB → 使用 `send_with_transfer()`
    /// - PostMessage + 数据 < 10KB → 使用普通 `send()`
    /// - 其他 Lane → 使用普通 `send()`
    ///
    /// # 参数
    /// - `data`: 要发送的数据
    ///
    /// # 返回
    /// - `Ok(())`: 发送成功
    /// - `Err`: 发送失败
    pub async fn send_auto(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage { .. } if should_use_transfer(data.len()) => {
                // 大数据使用 Transferable Objects
                self.send_with_transfer(data).await
            }
            _ => {
                // 其他情况使用普通 send
                self.send(data).await
            }
        }
    }

    /// 接收消息
    pub async fn recv(&self) -> Option<Bytes> {
        use futures::StreamExt;

        match self {
            DataLane::PostMessage {
                rx, payload_type, ..
            } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "PostMessage Lane (DOM) 接收消息: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );
                Some(data)
            }

            DataLane::WebRtcDataChannel {
                rx, payload_type, ..
            } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "WebRTC DataChannel Lane 接收消息: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );
                Some(data)
            }

            DataLane::WebRtcMediaTrack { rx, track_id, .. } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "WebRTC MediaTrack Lane 接收媒体帧: track_id={}, size={} bytes",
                    track_id,
                    data.len()
                );
                Some(data)
            }
        }
    }

    /// 获取 PayloadType
    pub fn payload_type(&self) -> PayloadType {
        match self {
            DataLane::PostMessage { payload_type, .. } => *payload_type,
            DataLane::WebRtcDataChannel { payload_type, .. } => *payload_type,
            DataLane::WebRtcMediaTrack { .. } => PayloadType::MediaRtp,
        }
    }
}
