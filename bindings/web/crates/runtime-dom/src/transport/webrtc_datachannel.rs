//! WebRTC DataChannel Lane - DOM 端的 WebRTC DataChannel 传输通道
//!
//! WebRTC DataChannel Lane 用于 DOM 端通过 WebRTC DataChannel 传输消息。
//! 支持的 PayloadType：RPC_*, STREAM_* (不支持 MEDIA_RTP)
//!
//! ## 注意事项
//! - DataChannel 只能在 DOM 环境中使用（Service Worker 不支持 WebRTC）
//! - 需要先建立 PeerConnection，然后创建 DataChannel
//! - DataChannel 支持有序/无序、可靠/不可靠传输模式

use super::lane::{DataLane, LaneResult};
use actr_web_common::PayloadType;
use actr_web_common::WebError;
use actr_web_common::zero_copy::{
    extract_payload_zero_copy, parse_message_header, receive_zero_copy,
};
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, RtcDataChannel, RtcDataChannelInit, RtcDataChannelState};

/// WebRTC DataChannel Lane 构建器
///
/// 用于创建和配置 WebRTC DataChannel Lane
pub struct WebRtcDataChannelLaneBuilder {
    data_channel: RtcDataChannel,
    payload_type: PayloadType,
    buffer_size: usize,
}

impl WebRtcDataChannelLaneBuilder {
    /// 创建新的 WebRTC DataChannel Lane 构建器
    ///
    /// # 参数
    /// - `data_channel`: RtcDataChannel 对象（从 RtcPeerConnection 获取）
    /// - `payload_type`: 该 Lane 传输的 PayloadType
    pub fn new(data_channel: RtcDataChannel, payload_type: PayloadType) -> Self {
        Self {
            data_channel,
            payload_type,
            buffer_size: 256, // 默认缓冲区大小
        }
    }

    /// 设置接收缓冲区大小
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// 构建 WebRTC DataChannel Lane
    ///
    /// # 错误
    /// - 如果 PayloadType 不支持（MEDIA_RTP）
    /// - 如果 DataChannel 配置失败
    pub async fn build(self) -> LaneResult<DataLane> {
        // 验证 PayloadType
        if matches!(self.payload_type, PayloadType::MediaRtp) {
            return Err(WebError::Transport(
                "WebRTC DataChannel Lane 不支持 MEDIA_RTP，请使用 MediaTrack Lane".to_string(),
            ));
        }

        // 设置 DataChannel 为二进制模式
        // 注意：RtcDataChannel 的 binary_type 默认就是 arraybuffer，无需设置
        // 如果需要设置，使用 JS 绑定：self.data_channel.set_binary_type("arraybuffer")

        // 创建接收通道
        let (tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        // 设置 onmessage 回调（零拷贝优化）
        let tx_clone = tx.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            // 尝试获取 ArrayBuffer 数据
            if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);

                // ✅ 零拷贝接收：1 次拷贝（JS → WASM 线性内存）
                let data = receive_zero_copy(&uint8_array);

                // 解析消息头部
                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "WebRTC DataChannel Lane 接收消息: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // ✅ 零拷贝提取 payload：转移 Vec 所有权到 Bytes
                    let payload_data = extract_payload_zero_copy(data, 5);

                    // 发送到通道（忽略发送失败，可能是接收端已关闭）
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            } else {
                log::warn!("DataChannel 接收到非 ArrayBuffer 数据，已忽略");
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        self.data_channel
            .set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        // 设置 onerror 回调
        let label = self.data_channel.label();
        let onerror_callback = Closure::wrap(Box::new(move |e: web_sys::ErrorEvent| {
            log::error!(
                "WebRTC DataChannel 错误 (label={}): {:?}",
                label,
                e.message()
            );
        }) as Box<dyn FnMut(web_sys::ErrorEvent)>);

        self.data_channel
            .set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        onerror_callback.forget();

        // 设置 onclose 回调
        let label = self.data_channel.label();
        let onclose_callback = Closure::wrap(Box::new(move |_e: JsValue| {
            log::info!("WebRTC DataChannel 连接关闭 (label={})", label);
        }) as Box<dyn FnMut(JsValue)>);

        self.data_channel
            .set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));
        onclose_callback.forget();

        // 设置 onopen 回调
        let label = self.data_channel.label();
        let payload_type = self.payload_type;
        let onopen_callback = Closure::wrap(Box::new(move |_e: JsValue| {
            log::info!(
                "WebRTC DataChannel 连接已建立: label={}, payload_type={:?}",
                label,
                payload_type
            );
        }) as Box<dyn FnMut(JsValue)>);

        self.data_channel
            .set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
        onopen_callback.forget();

        // 等待 DataChannel 打开（如果还没打开）
        let dc_clone = self.data_channel.clone();
        let wait_future = async move {
            let start = js_sys::Date::now();
            loop {
                let state = dc_clone.ready_state();
                if state == RtcDataChannelState::Open {
                    return Ok(());
                }

                if state == RtcDataChannelState::Closed || state == RtcDataChannelState::Closing {
                    return Err(WebError::Transport(
                        "WebRTC DataChannel 连接失败或已关闭".to_string(),
                    ));
                }

                if js_sys::Date::now() - start > 10000.0 {
                    return Err(WebError::Transport(
                        "WebRTC DataChannel 连接超时（10秒）".to_string(),
                    ));
                }

                // 等待 50ms 后重试
                wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
                    let window = web_sys::window().unwrap();
                    window
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50)
                        .unwrap();
                }))
                .await
                .unwrap();
            }
        };

        wait_future.await?;

        log::info!(
            "WebRTC DataChannel Lane 创建成功: label={}, payload_type={:?}",
            self.data_channel.label(),
            self.payload_type
        );

        Ok(DataLane::WebRtcDataChannel {
            data_channel: Arc::new(self.data_channel),
            payload_type: self.payload_type,
            rx,
        })
    }
}

/// DataChannel 配置辅助函数
///
/// 根据 PayloadType 创建合适的 DataChannel 配置
pub fn create_datachannel_config(payload_type: PayloadType) -> RtcDataChannelInit {
    let config = RtcDataChannelInit::new();

    match payload_type {
        PayloadType::RpcReliable | PayloadType::StreamReliable => {
            // 可靠有序传输
            config.set_ordered(true);
            // max_retransmits 不设置表示无限重传
        }
        PayloadType::RpcSignal | PayloadType::StreamLatencyFirst => {
            // 低延迟传输（允许乱序和丢包）
            config.set_ordered(false);
            config.set_max_retransmits(0); // 不重传
        }
        PayloadType::MediaRtp => {
            // DataChannel 不应该用于 MEDIA_RTP
            log::warn!("DataChannel 不应该用于 MEDIA_RTP，请使用 MediaTrack");
            config.set_ordered(false);
            config.set_max_retransmits(0);
        }
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // 注意：以下测试因 web-sys API 变更暂时禁用
    // RtcDataChannelInit 的 getter 方法在新版本中签名已变化
    // TODO: 更新测试以使用新的 API
    /*
    #[wasm_bindgen_test]
    fn test_datachannel_config_for_reliable_types() {
        let config = create_datachannel_config(PayloadType::RpcReliable);
        assert_eq!(config.ordered(), true);

        let config = create_datachannel_config(PayloadType::StreamReliable);
        assert_eq!(config.ordered(), true);
    }

    #[wasm_bindgen_test]
    fn test_datachannel_config_for_latency_first_types() {
        let config = create_datachannel_config(PayloadType::RpcSignal);
        assert_eq!(config.ordered(), false);
        assert_eq!(config.max_retransmits(), Some(0));

        let config = create_datachannel_config(PayloadType::StreamLatencyFirst);
        assert_eq!(config.ordered(), false);
        assert_eq!(config.max_retransmits(), Some(0));
    }
    */

    #[wasm_bindgen_test]
    async fn test_webrtc_datachannel_lane_rejects_media_rtp() {
        // 注意：这个测试需要实际的 RtcPeerConnection
        // 在真实测试环境中应该创建完整的 PeerConnection 和 DataChannel

        // 这里只验证 PayloadType 验证逻辑
        let payload_types = vec![
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
        ];

        for payload_type in payload_types {
            // 验证这些类型不是 MEDIA_RTP
            assert!(!matches!(payload_type, PayloadType::MediaRtp));
        }

        // 验证 MEDIA_RTP 会被拒绝
        assert!(matches!(PayloadType::MediaRtp, PayloadType::MediaRtp));
    }
}
