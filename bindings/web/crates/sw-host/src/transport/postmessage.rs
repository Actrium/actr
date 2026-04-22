//! PostMessage lane for the Service Worker side.
//!
//! Used by the Service Worker to send messages to the DOM side.

use super::WirePool;
use super::lane::{DataLane, LaneResult, PortFailureNotifier};
use actr_web_common::zero_copy::{
    extract_payload_zero_copy, parse_message_header, receive_zero_copy,
};
use actr_web_common::{ConnType, PayloadType};
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, MessagePort};

/// Builder for the Service Worker-side PostMessage lane.
pub struct PostMessageLaneBuilder {
    port: MessagePort,
    payload_type: PayloadType,
    buffer_size: usize,
    wire_pool: Option<Arc<WirePool>>,
    conn_type: ConnType,
}

impl PostMessageLaneBuilder {
    /// Create a new PostMessage lane builder.
    ///
    /// # Parameters
    /// - `port`: `MessagePort` passed from the DOM side
    /// - `payload_type`: payload type carried by this lane
    pub fn new(port: MessagePort, payload_type: PayloadType) -> Self {
        Self {
            port,
            payload_type,
            buffer_size: 256,
            wire_pool: None,
            conn_type: ConnType::WebRTC, // Defaults to WebRTC because PostMessage is mainly used for it.
        }
    }

    /// Attach a WirePool reference for failure notification.
    pub fn with_wire_pool(mut self, wire_pool: Arc<WirePool>, conn_type: ConnType) -> Self {
        self.wire_pool = Some(wire_pool);
        self.conn_type = conn_type;
        self
    }

    /// Set the receive buffer size.
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Build the PostMessage lane.
    pub fn build(self) -> LaneResult<DataLane> {
        // Create the receive channel.
        let (tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        // Install the `onmessage` callback with zero-copy handling.
        let tx_clone = tx.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            // Try to read the payload as a `Uint8Array`.
            if let Ok(uint8_array) = e.data().dyn_into::<js_sys::Uint8Array>() {
                // Zero-copy receive: one copy from JS into WASM linear memory.
                let data = receive_zero_copy(&uint8_array);

                // Parse the message header.
                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "PostMessage lane (SW) received message: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // Extract the payload by transferring Vec ownership into Bytes.
                    let payload_data = extract_payload_zero_copy(data, 5);
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            } else if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                // Compatibility path for `ArrayBuffer`.
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);

                // Zero-copy receive.
                let data = receive_zero_copy(&uint8_array);

                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "PostMessage lane (SW) received message: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // Extract the payload without an extra copy.
                    let payload_data = extract_payload_zero_copy(data, 5);
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        self.port
            .set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        // Start the `MessagePort`.
        self.port.start();

        log::info!(
            "PostMessage lane (SW) created: payload_type={:?}",
            self.payload_type
        );

        // Create a failure notifier if a WirePool is attached.
        let failure_notifier = self
            .wire_pool
            .map(|pool| PortFailureNotifier::new(pool, self.conn_type));

        Ok(DataLane::PostMessage {
            port: Arc::new(self.port),
            payload_type: self.payload_type,
            rx,
            failure_notifier,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // ===== Basic PostMessageLaneBuilder tests =====

    #[wasm_bindgen_test]
    fn test_postmessage_lane_builder_new() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let builder = PostMessageLaneBuilder::new(port, PayloadType::RpcReliable);

        assert_eq!(builder.payload_type, PayloadType::RpcReliable);
        assert_eq!(builder.buffer_size, 256);
        assert!(builder.wire_pool.is_none());
        assert_eq!(builder.conn_type, ConnType::WebRTC);
    }

    #[wasm_bindgen_test]
    fn test_postmessage_lane_builder_with_wire_pool() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let wire_pool = Arc::new(WirePool::new());
        let builder = PostMessageLaneBuilder::new(port, PayloadType::MediaRtp)
            .with_wire_pool(wire_pool.clone(), ConnType::WebSocket);

        assert!(builder.wire_pool.is_some());
        assert_eq!(builder.conn_type, ConnType::WebSocket);
    }

    #[wasm_bindgen_test]
    fn test_postmessage_lane_builder_buffer_size() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let builder = PostMessageLaneBuilder::new(port, PayloadType::RpcSignal).buffer_size(512);

        assert_eq!(builder.buffer_size, 512);
    }

    #[wasm_bindgen_test]
    fn test_postmessage_lane_builder_chaining() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let wire_pool = Arc::new(WirePool::new());
        let builder = PostMessageLaneBuilder::new(port, PayloadType::StreamReliable)
            .with_wire_pool(wire_pool.clone(), ConnType::WebRTC)
            .buffer_size(1024);

        assert_eq!(builder.buffer_size, 1024);
        assert!(builder.wire_pool.is_some());
        assert_eq!(builder.conn_type, ConnType::WebRTC);
        assert_eq!(builder.payload_type, PayloadType::StreamReliable);
    }

    #[wasm_bindgen_test]
    fn test_postmessage_lane_builder_default_values() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let builder = PostMessageLaneBuilder::new(port, PayloadType::StreamLatencyFirst);

        assert_eq!(builder.buffer_size, 256);
        assert!(builder.wire_pool.is_none());
        assert_eq!(builder.conn_type, ConnType::WebRTC);
    }

    // ===== PostMessageLaneBuilder build() tests =====

    #[wasm_bindgen_test]
    async fn test_postmessage_lane_builder_build_basic() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let builder = PostMessageLaneBuilder::new(port, PayloadType::RpcReliable);
        let result = builder.build();

        assert!(result.is_ok());

        let lane = result.unwrap();
        assert_eq!(lane.payload_type(), PayloadType::RpcReliable);
    }

    #[wasm_bindgen_test]
    async fn test_postmessage_lane_builder_build_with_notifier() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let wire_pool = Arc::new(WirePool::new());
        let builder = PostMessageLaneBuilder::new(port, PayloadType::MediaRtp)
            .with_wire_pool(wire_pool, ConnType::WebRTC);

        let result = builder.build();
        assert!(result.is_ok());

        let lane = result.unwrap();
        match lane {
            DataLane::PostMessage {
                failure_notifier, ..
            } => {
                assert!(failure_notifier.is_some());
            }
            _ => panic!("Expected PostMessage lane"),
        }
    }

    #[wasm_bindgen_test]
    async fn test_postmessage_lane_builder_build_without_notifier() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let builder = PostMessageLaneBuilder::new(port, PayloadType::RpcSignal);
        let result = builder.build();

        assert!(result.is_ok());

        let lane = result.unwrap();
        match lane {
            DataLane::PostMessage {
                failure_notifier, ..
            } => {
                assert!(failure_notifier.is_none());
            }
            _ => panic!("Expected PostMessage lane"),
        }
    }

    // ===== Different PayloadType tests =====

    #[wasm_bindgen_test]
    fn test_postmessage_builder_all_payload_types() {
        let payload_types = vec![
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
            PayloadType::MediaRtp,
        ];

        for pt in payload_types {
            let channel = web_sys::MessageChannel::new().unwrap();
            let port = channel.port1();

            let builder = PostMessageLaneBuilder::new(port, pt);
            assert_eq!(builder.payload_type, pt);
        }
    }

    #[wasm_bindgen_test]
    fn test_postmessage_builder_conn_types() {
        let channel1 = web_sys::MessageChannel::new().unwrap();
        let port1 = channel1.port1();
        let wire_pool1 = Arc::new(WirePool::new());
        let builder1 = PostMessageLaneBuilder::new(port1, PayloadType::RpcReliable)
            .with_wire_pool(wire_pool1, ConnType::WebSocket);
        assert_eq!(builder1.conn_type, ConnType::WebSocket);

        let channel2 = web_sys::MessageChannel::new().unwrap();
        let port2 = channel2.port1();
        let wire_pool2 = Arc::new(WirePool::new());
        let builder2 = PostMessageLaneBuilder::new(port2, PayloadType::MediaRtp)
            .with_wire_pool(wire_pool2, ConnType::WebRTC);
        assert_eq!(builder2.conn_type, ConnType::WebRTC);
    }

    #[wasm_bindgen_test]
    fn test_postmessage_builder_multiple_buffer_sizes() {
        let sizes = vec![128, 256, 512, 1024, 2048];

        for size in sizes {
            let channel = web_sys::MessageChannel::new().unwrap();
            let port = channel.port1();

            let builder =
                PostMessageLaneBuilder::new(port, PayloadType::RpcReliable).buffer_size(size);

            assert_eq!(builder.buffer_size, size);
        }
    }

    #[wasm_bindgen_test]
    async fn test_postmessage_lane_clone() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port = channel.port1();

        let builder = PostMessageLaneBuilder::new(port, PayloadType::StreamReliable);
        let lane1 = builder.build().unwrap();
        let lane2 = lane1.clone();

        match (&lane1, &lane2) {
            (
                DataLane::PostMessage { port: port1, .. },
                DataLane::PostMessage { port: port2, .. },
            ) => {
                assert!(Arc::ptr_eq(port1, port2));
            }
            _ => panic!("Expected PostMessage lanes"),
        }
    }
}
