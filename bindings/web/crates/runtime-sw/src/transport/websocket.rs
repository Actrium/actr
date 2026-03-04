//! WebSocket Lane - Service Worker 端的 WebSocket 传输通道
//!
//! WebSocket Lane 用于 Service Worker 端通过 WebSocket 连接传输消息。
//! 支持的 PayloadType：RPC_*, STREAM_* (不支持 MEDIA_RTP)

use super::lane::{DataLane, LaneResult};
use actr_web_common::zero_copy::{
    extract_payload_zero_copy, parse_message_header, receive_zero_copy,
};
use actr_web_common::{PayloadType, WebError};

use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

/// WebSocket Lane 构建器
///
/// 用于创建和配置 WebSocket Lane
pub struct WebSocketLaneBuilder {
    url: String,
    payload_type: PayloadType,
    buffer_size: usize,
}

impl WebSocketLaneBuilder {
    /// 创建新的 WebSocket Lane 构建器
    ///
    /// # 参数
    /// - `url`: WebSocket 服务器 URL (ws:// 或 wss://)
    /// - `payload_type`: 该 Lane 传输的 PayloadType
    pub fn new(url: impl Into<String>, payload_type: PayloadType) -> Self {
        Self {
            url: url.into(),
            payload_type,
            buffer_size: 256, // 默认缓冲区大小
        }
    }

    /// 设置接收缓冲区大小
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// 构建 WebSocket Lane
    ///
    /// # 错误
    /// - 如果 PayloadType 不支持（MEDIA_RTP）
    /// - 如果 WebSocket 创建失败
    pub async fn build(self) -> LaneResult<DataLane> {
        // 验证 PayloadType
        if matches!(self.payload_type, PayloadType::MediaRtp) {
            return Err(WebError::Transport(
                "WebSocket Lane 不支持 MEDIA_RTP，请使用 MediaTrack Lane".to_string(),
            ));
        }

        // 创建 WebSocket 连接
        let ws = WebSocket::new(&self.url)
            .map_err(|e| WebError::Transport(format!("WebSocket 创建失败: {:?}", e)))?;

        // 设置为二进制模式
        ws.set_binary_type(BinaryType::Arraybuffer);

        // 创建接收通道
        let (tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        // 设置 onmessage 回调（零拷贝优化）
        let tx_clone = tx.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);

                // ✅ 零拷贝接收：1 次拷贝（JS → WASM 线性内存）
                let data = receive_zero_copy(&uint8_array);

                // 解析消息头部
                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "WebSocket Lane 接收消息: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // ✅ 零拷贝提取 payload：转移 Vec 所有权到 Bytes
                    let payload_data = extract_payload_zero_copy(data, 5);

                    // 发送到通道（忽略发送失败，可能是接收端已关闭）
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        ws.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        // 设置 onerror 回调
        // 注意：WebSocket 的 error 事件是 Event 类型（非 ErrorEvent），没有 .message 属性
        let onerror_callback = Closure::wrap(Box::new(move |_e: web_sys::Event| {
            log::error!("WebSocket 连接错误");
        }) as Box<dyn FnMut(web_sys::Event)>);

        ws.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        onerror_callback.forget();

        // 设置 onclose 回调
        let onclose_callback = Closure::wrap(Box::new(move |e: CloseEvent| {
            log::info!(
                "WebSocket 连接关闭: code={}, reason={}, clean={}",
                e.code(),
                e.reason(),
                e.was_clean()
            );
        }) as Box<dyn FnMut(CloseEvent)>);

        ws.set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));
        onclose_callback.forget();

        // 设置 onopen 回调
        let ws_clone = ws.clone();
        let payload_type = self.payload_type;
        let onopen_callback = Closure::wrap(Box::new(move |_e: JsValue| {
            log::info!(
                "WebSocket 连接已建立: url={}, payload_type={:?}",
                ws_clone.url(),
                payload_type
            );
        }) as Box<dyn FnMut(JsValue)>);

        ws.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
        onopen_callback.forget();

        // 等待连接建立（最多 5 秒）
        let ws_clone = ws.clone();
        let connect_future = async move {
            let start = js_sys::Date::now();
            loop {
                if ws_clone.ready_state() == WebSocket::OPEN {
                    return Ok(());
                }

                if ws_clone.ready_state() == WebSocket::CLOSED
                    || ws_clone.ready_state() == WebSocket::CLOSING
                {
                    return Err(WebError::Transport(
                        "WebSocket 连接失败或已关闭".to_string(),
                    ));
                }

                if js_sys::Date::now() - start > 5000.0 {
                    return Err(WebError::Transport("WebSocket 连接超时（5秒）".to_string()));
                }

                // 等待 10ms 后重试（使用 gloo_timers，兼容 Service Worker 环境）
                gloo_timers::future::TimeoutFuture::new(10).await;
            }
        };

        connect_future.await?;

        log::info!(
            "WebSocket Lane 创建成功: url={}, payload_type={:?}",
            self.url,
            self.payload_type
        );

        Ok(DataLane::WebSocket {
            ws: Arc::new(ws),
            payload_type: self.payload_type,
            rx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_websocket_lane_builder_rejects_media_rtp() {
        let result = WebSocketLaneBuilder::new("ws://localhost:8080", PayloadType::MediaRtp)
            .build()
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不支持 MEDIA_RTP"));
    }

    #[wasm_bindgen_test]
    async fn test_websocket_lane_accepts_rpc_types() {
        // 注意：这个测试需要实际的 WebSocket 服务器
        // 在真实测试环境中应该使用 mock 服务器
        let payload_types = vec![
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
        ];

        for payload_type in payload_types {
            let builder = WebSocketLaneBuilder::new("ws://localhost:8080", payload_type);
            // 由于没有实际服务器，这里只验证 builder 的创建
            assert_eq!(builder.payload_type, payload_type);
        }
    }

    // ===== WebSocketLaneBuilder 基本测试 =====

    #[test]
    fn test_websocket_lane_builder_new() {
        let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcReliable);

        assert_eq!(builder.url, "ws://test.com");
        assert_eq!(builder.payload_type, PayloadType::RpcReliable);
        assert_eq!(builder.buffer_size, 256);
    }

    #[test]
    fn test_websocket_lane_builder_new_with_wss() {
        let builder = WebSocketLaneBuilder::new("wss://secure.test.com", PayloadType::RpcSignal);

        assert_eq!(builder.url, "wss://secure.test.com");
        assert_eq!(builder.payload_type, PayloadType::RpcSignal);
    }

    #[test]
    fn test_websocket_lane_builder_buffer_size() {
        let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::StreamReliable)
            .buffer_size(512);

        assert_eq!(builder.buffer_size, 512);
    }

    #[test]
    fn test_websocket_lane_builder_chaining() {
        let builder =
            WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcReliable).buffer_size(1024);

        assert_eq!(builder.buffer_size, 1024);
        assert_eq!(builder.url, "ws://test.com");
        assert_eq!(builder.payload_type, PayloadType::RpcReliable);
    }

    #[test]
    fn test_websocket_lane_builder_default_buffer_size() {
        let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::StreamLatencyFirst);

        assert_eq!(builder.buffer_size, 256);
    }

    // ===== PayloadType 支持测试 =====

    #[test]
    fn test_websocket_lane_builder_rpc_reliable() {
        let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcReliable);
        assert_eq!(builder.payload_type, PayloadType::RpcReliable);
    }

    #[test]
    fn test_websocket_lane_builder_rpc_signal() {
        let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcSignal);
        assert_eq!(builder.payload_type, PayloadType::RpcSignal);
    }

    #[test]
    fn test_websocket_lane_builder_stream_reliable() {
        let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::StreamReliable);
        assert_eq!(builder.payload_type, PayloadType::StreamReliable);
    }

    #[test]
    fn test_websocket_lane_builder_stream_latency_first() {
        let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::StreamLatencyFirst);
        assert_eq!(builder.payload_type, PayloadType::StreamLatencyFirst);
    }

    // ===== URL 格式测试 =====

    #[test]
    fn test_websocket_lane_builder_url_formats() {
        let urls = vec![
            "ws://localhost:8080",
            "wss://secure.example.com",
            "ws://192.168.1.1:9000",
            "wss://example.com/path/to/endpoint",
        ];

        for url in urls {
            let builder = WebSocketLaneBuilder::new(url, PayloadType::RpcReliable);
            assert_eq!(builder.url, url);
        }
    }

    #[test]
    fn test_websocket_lane_builder_string_ownership() {
        let url = String::from("ws://test.com");
        let builder = WebSocketLaneBuilder::new(url.clone(), PayloadType::RpcReliable);

        assert_eq!(builder.url, url);
        // Verify the original url is still valid
        assert_eq!(url, "ws://test.com");
    }

    #[test]
    fn test_websocket_lane_builder_str_slice() {
        let url: &str = "ws://test.com";
        let builder = WebSocketLaneBuilder::new(url, PayloadType::RpcSignal);

        assert_eq!(builder.url, "ws://test.com");
    }

    // ===== Buffer Size 变化测试 =====

    #[test]
    fn test_websocket_lane_builder_multiple_buffer_sizes() {
        let sizes = vec![128, 256, 512, 1024, 2048, 4096];

        for size in sizes {
            let builder = WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcReliable)
                .buffer_size(size);

            assert_eq!(builder.buffer_size, size);
        }
    }

    #[test]
    fn test_websocket_lane_builder_zero_buffer_size() {
        let builder =
            WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcReliable).buffer_size(0);

        assert_eq!(builder.buffer_size, 0);
    }

    // ===== Builder 模式完整性测试 =====

    #[test]
    fn test_websocket_lane_builder_immutability() {
        let builder1 = WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcReliable);
        let builder2 = builder1.buffer_size(512);

        // builder2 是一个新的 builder，因为 buffer_size 消费了 builder1
        assert_eq!(builder2.buffer_size, 512);
    }
}
