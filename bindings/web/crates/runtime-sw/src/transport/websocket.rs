//! WebSocket transport lane for the Service Worker runtime.
//!
//! This lane carries messages over WebSocket on the Service Worker side.
//! Supported payload types: `RPC_*`, `STREAM_*` (but not `MEDIA_RTP`).

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

/// Builder for a WebSocket lane.
///
/// Creates and configures a `WebSocket`-backed `DataLane`.
pub struct WebSocketLaneBuilder {
    url: String,
    payload_type: PayloadType,
    buffer_size: usize,
}

impl WebSocketLaneBuilder {
    /// Create a new WebSocket lane builder.
    ///
    /// # Parameters
    /// - `url`: WebSocket server URL (`ws://` or `wss://`)
    /// - `payload_type`: payload type carried by this lane
    pub fn new(url: impl Into<String>, payload_type: PayloadType) -> Self {
        Self {
            url: url.into(),
            payload_type,
            buffer_size: 256, // Default buffer size.
        }
    }

    /// Set the receive buffer size.
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Build the WebSocket lane.
    ///
    /// # Errors
    /// - If the payload type is unsupported (`MEDIA_RTP`)
    /// - If creating the WebSocket fails
    pub async fn build(self) -> LaneResult<DataLane> {
        // Validate the payload type.
        if matches!(self.payload_type, PayloadType::MediaRtp) {
            return Err(WebError::Transport(
                "WebSocket lanes do not support MEDIA_RTP; use a MediaTrack lane".to_string(),
            ));
        }

        // Create the WebSocket connection.
        let ws = WebSocket::new(&self.url)
            .map_err(|e| WebError::Transport(format!("Failed to create WebSocket: {:?}", e)))?;

        // Use binary mode.
        ws.set_binary_type(BinaryType::Arraybuffer);

        // Create the receive channel.
        let (tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        // Install the `onmessage` callback using zero-copy helpers.
        let tx_clone = tx.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);

                // Zero-copy receive with a single copy from JS into WASM memory.
                let data = receive_zero_copy(&uint8_array);

                // Parse the message header.
                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "WebSocket lane received message: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // Extract the payload by transferring Vec ownership into Bytes.
                    let payload_data = extract_payload_zero_copy(data, 5);

                    // Ignore send failures because the receiver may already be closed.
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        ws.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        // Install the `onerror` callback.
        // The WebSocket `error` event is an `Event`, not an `ErrorEvent`.
        let onerror_callback = Closure::wrap(Box::new(move |_e: web_sys::Event| {
            log::error!("WebSocket connection error");
        }) as Box<dyn FnMut(web_sys::Event)>);

        ws.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        onerror_callback.forget();

        // Install the `onclose` callback.
        let onclose_callback = Closure::wrap(Box::new(move |e: CloseEvent| {
            log::info!(
                "WebSocket connection closed: code={}, reason={}, clean={}",
                e.code(),
                e.reason(),
                e.was_clean()
            );
        }) as Box<dyn FnMut(CloseEvent)>);

        ws.set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));
        onclose_callback.forget();

        // Install the `onopen` callback.
        let ws_clone = ws.clone();
        let payload_type = self.payload_type;
        let onopen_callback = Closure::wrap(Box::new(move |_e: JsValue| {
            log::info!(
                "WebSocket connection established: url={}, payload_type={:?}",
                ws_clone.url(),
                payload_type
            );
        }) as Box<dyn FnMut(JsValue)>);

        ws.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
        onopen_callback.forget();

        // Wait for the connection to open, up to 5 seconds.
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
                        "WebSocket connection failed or closed".to_string(),
                    ));
                }

                if js_sys::Date::now() - start > 5000.0 {
                    return Err(WebError::Transport("WebSocket connection timed out (5s)".to_string()));
                }

                // Retry after 10 ms with `gloo_timers`, which works in Service Workers.
                gloo_timers::future::TimeoutFuture::new(10).await;
            }
        };

        connect_future.await?;

        log::info!(
            "WebSocket lane created: url={}, payload_type={:?}",
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("do not support MEDIA_RTP")
        );
    }

    #[wasm_bindgen_test]
    async fn test_websocket_lane_accepts_rpc_types() {
        // This test would require a real WebSocket server.
        // A mock server should be used in a full integration environment.
        let payload_types = vec![
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
        ];

        for payload_type in payload_types {
            let builder = WebSocketLaneBuilder::new("ws://localhost:8080", payload_type);
            // Without a real server, this only verifies builder construction.
            assert_eq!(builder.payload_type, payload_type);
        }
    }

    // ===== Basic WebSocketLaneBuilder tests =====

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

    // ===== PayloadType support tests =====

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

    // ===== URL format tests =====

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

    // ===== Buffer-size variation tests =====

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

    // ===== Builder-pattern integrity tests =====

    #[test]
    fn test_websocket_lane_builder_immutability() {
        let builder1 = WebSocketLaneBuilder::new("ws://test.com", PayloadType::RpcReliable);
        let builder2 = builder1.buffer_size(512);

        // `builder2` is a new builder because `buffer_size` consumes `builder1`.
        assert_eq!(builder2.buffer_size, 512);
    }
}
