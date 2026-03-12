//! Data transport lanes for the Service Worker runtime.
//!
//! The Service Worker side contains:
//! - `WebSocket` lanes for server communication
//! - `PostMessage` lanes for DOM communication

use super::WirePool;
use actr_web_common::zero_copy::{
    construct_message_header, construct_message_zero_copy, send_with_transfer, send_zero_copy,
    should_use_transfer,
};
use actr_web_common::{ConnType, PayloadType, WebError, WebResult};
use bytes::Bytes;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{MessagePort, WebSocket};

/// Transport-layer operation result in the Service Worker runtime.
pub type LaneResult<T> = WebResult<T>;

/// Notifies the WirePool when a port fails.
///
/// If sending on a `MessagePort` fails, the notifier marks the associated
/// connection as failed in the `WirePool`.
#[derive(Clone)]
pub struct PortFailureNotifier {
    wire_pool: Arc<WirePool>,
    conn_type: ConnType,
}

impl PortFailureNotifier {
    /// Create a new failure notifier.
    pub fn new(wire_pool: Arc<WirePool>, conn_type: ConnType) -> Self {
        Self {
            wire_pool,
            conn_type,
        }
    }

    /// Report that the port has failed.
    pub fn notify_port_failed(&self) {
        log::warn!(
            "[PortFailureNotifier] Notifying port failure: {:?}",
            self.conn_type
        );

        self.wire_pool.mark_connection_failed(self.conn_type);
    }
}

/// Data transport lane used by the Service Worker runtime.
#[derive(Clone)]
pub enum DataLane {
    /// WebSocket Lane
    ///
    /// WebSocket connection to the server.
    /// Supports `RPC_*` and `STREAM_*`, but not `MEDIA_RTP`.
    WebSocket {
        ws: Arc<WebSocket>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },

    /// PostMessage Lane
    ///
    /// Message channel used to communicate with the DOM side.
    /// Supports all payload types.
    PostMessage {
        port: Arc<MessagePort>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
        failure_notifier: Option<PortFailureNotifier>,
    },
}

impl DataLane {
    /// Send a message using zero-copy helpers.
    pub async fn send(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::WebSocket {
                ws, payload_type, ..
            } => {
                // Build the message with zero-copy helper functions.
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // Send the Bytes slice directly because WebSocket accepts `&[u8]`.
                ws.send_with_u8_array(&msg)
                    .map_err(|e| WebError::Transport(format!("WebSocket send failed: {:?}", e)))?;

                log::trace!(
                    "WebSocket lane sent message: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );

                Ok(())
            }

            DataLane::PostMessage {
                port,
                payload_type,
                failure_notifier,
                ..
            } => {
                // Build the message with zero-copy helper functions.
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // Zero-copy send by creating a WASM memory view.
                let js_view = send_zero_copy(&msg);

                // Attempt the send and capture failures.
                match port.post_message(&js_view.into()) {
                    Ok(_) => {
                        log::trace!(
                            "PostMessage lane sent message: payload_type={:?}, size={} bytes",
                            payload_type,
                            data.len()
                        );
                        Ok(())
                    }
                    Err(e) => {
                        // The MessagePort appears to be dead.
                        log::error!("PostMessage send failed (port may be dead): {:?}", e);

                        // Notify the WirePool that the connection failed.
                        if let Some(notifier) = failure_notifier {
                            notifier.notify_port_failed();
                        }

                        Err(WebError::Transport(format!("PostMessage failed: {:?}", e)))
                    }
                }
            }
        }
    }

    /// Send a message with transferable objects (PostMessage only).
    ///
    /// **Transferable objects**:
    /// - Transfer `ArrayBuffer` ownership instead of copying it
    /// - Best for large payloads (>10KB)
    /// - Supported only by `PostMessage` lanes, not `WebSocket`
    ///
    /// # Parameters
    /// - `data`: data to send
    ///
    /// # Returns
    /// - `Ok(())` if the send succeeds
    /// - `Err` if the send fails or the lane type does not support it
    ///
    /// # Guidance
    /// - Use this for large payloads (>10KB)
    /// - Use `send()` for smaller payloads (<10KB)
    /// - Or call `send_auto()` to pick automatically
    pub async fn send_with_transfer(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage {
                port,
                payload_type,
                failure_notifier,
                ..
            } => {
                // Build the message with zero-copy helper functions.
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // Send with transferable objects.
                let (js_view, transfer_list) = send_with_transfer(&msg);

                // Call `postMessage(message, transferList)` via the lower-level wasm-bindgen API.
                let post_message_fn =
                    js_sys::Reflect::get(port.as_ref(), &JsValue::from_str("postMessage"))
                        .map_err(|e| {
                            WebError::Transport(format!("Failed to get postMessage: {:?}", e))
                        })?;

                let func: &js_sys::Function = post_message_fn.unchecked_ref();
                let result = js_sys::Reflect::apply(
                    func,
                    port.as_ref(),
                    &js_sys::Array::of2(&js_view.into(), &transfer_list),
                );

                match result {
                    Ok(_) => {
                        log::trace!(
                            "PostMessage lane (SW) sent message with transfer: payload_type={:?}, size={} bytes",
                            payload_type,
                            data.len()
                        );
                        Ok(())
                    }
                    Err(e) => {
                        // The MessagePort appears to be dead.
                        log::error!(
                            "PostMessage with transfer failed (port may be dead): {:?}",
                            e
                        );

                        // Notify the WirePool that the connection failed.
                        if let Some(notifier) = failure_notifier {
                            notifier.notify_port_failed();
                        }

                        Err(WebError::Transport(format!(
                            "PostMessage with transfer failed: {:?}",
                            e
                        )))
                    }
                }
            }

            DataLane::WebSocket { .. } => {
                // WebSocket does not support transferable objects, so fall back to `send`.
                log::warn!("WebSocket does not support transferable objects; falling back to send");
                self.send(data).await
            }
        }
    }

    /// Pick a send strategy automatically based on payload size.
    ///
    /// **Decision rules**:
    /// - `PostMessage` + data >= 10KB -> `send_with_transfer()`
    /// - `PostMessage` + data < 10KB -> `send()`
    /// - `WebSocket` -> `send()`
    ///
    /// # Parameters
    /// - `data`: data to send
    ///
    /// # Returns
    /// - `Ok(())` if the send succeeds
    /// - `Err` if the send fails
    pub async fn send_auto(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage { .. } if should_use_transfer(data.len()) => {
                // Use transferable objects for large payloads.
                self.send_with_transfer(data).await
            }
            _ => {
                // Use the regular send path otherwise.
                self.send(data).await
            }
        }
    }

    /// Receive a message.
    pub async fn recv(&self) -> Option<Bytes> {
        use futures::StreamExt;

        match self {
            DataLane::WebSocket {
                rx, payload_type, ..
            } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "WebSocket lane received message: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );
                Some(data)
            }

            DataLane::PostMessage {
                rx, payload_type, ..
            } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "PostMessage lane received message: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );
                Some(data)
            }
        }
    }

    /// Get the lane payload type.
    pub fn payload_type(&self) -> PayloadType {
        match self {
            DataLane::WebSocket { payload_type, .. } => *payload_type,
            DataLane::PostMessage { payload_type, .. } => *payload_type,
        }
    }
}

impl std::fmt::Debug for DataLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataLane::WebSocket { payload_type, .. } => f
                .debug_struct("DataLane::WebSocket")
                .field("payload_type", payload_type)
                .finish_non_exhaustive(),
            DataLane::PostMessage { payload_type, .. } => f
                .debug_struct("DataLane::PostMessage")
                .field("payload_type", payload_type)
                .finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // ===== PortFailureNotifier tests =====

    #[test]
    fn test_port_failure_notifier_new() {
        let wire_pool = Arc::new(WirePool::new());
        let conn_type = ConnType::WebRTC;

        let notifier = PortFailureNotifier::new(wire_pool.clone(), conn_type);

        assert!(Arc::ptr_eq(&notifier.wire_pool, &wire_pool));
    }

    #[test]
    fn test_port_failure_notifier_clone() {
        let wire_pool = Arc::new(WirePool::new());
        let notifier1 = PortFailureNotifier::new(wire_pool, ConnType::WebSocket);
        let notifier2 = notifier1.clone();

        assert!(Arc::ptr_eq(&notifier1.wire_pool, &notifier2.wire_pool));
    }

    #[test]
    fn test_port_failure_notifier_conn_types() {
        let wire_pool = Arc::new(WirePool::new());

        let ws_notifier = PortFailureNotifier::new(wire_pool.clone(), ConnType::WebSocket);
        let rtc_notifier = PortFailureNotifier::new(wire_pool, ConnType::WebRTC);

        // Verify they can be created for different connection types
        assert_eq!(ws_notifier.conn_type, ConnType::WebSocket);
        assert_eq!(rtc_notifier.conn_type, ConnType::WebRTC);
    }

    // ===== DataLane payload type tests =====

    #[wasm_bindgen_test]
    fn test_data_lane_payload_type_websocket() {
        use web_sys::WebSocket;

        let ws = WebSocket::new("ws://test.com").unwrap();
        let (_tx, rx) = mpsc::unbounded();

        let lane = DataLane::WebSocket {
            ws: Arc::new(ws),
            payload_type: PayloadType::RpcReliable,
            rx: Arc::new(Mutex::new(rx)),
        };

        assert_eq!(lane.payload_type(), PayloadType::RpcReliable);
    }

    #[wasm_bindgen_test]
    fn test_data_lane_payload_type_postmessage() {
        use web_sys::MessageChannel;

        let channel = MessageChannel::new().unwrap();
        let port = channel.port1();
        let (_tx, rx) = mpsc::unbounded();

        let lane = DataLane::PostMessage {
            port: Arc::new(port),
            payload_type: PayloadType::StreamReliable,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: None,
        };

        assert_eq!(lane.payload_type(), PayloadType::StreamReliable);
    }

    #[wasm_bindgen_test]
    fn test_data_lane_payload_types_all() {
        use web_sys::WebSocket;

        let payload_types = vec![
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
            PayloadType::MediaRtp,
        ];

        for pt in payload_types {
            let ws = WebSocket::new("ws://test.com").unwrap();
            let (_tx, rx) = mpsc::unbounded();

            let lane = DataLane::WebSocket {
                ws: Arc::new(ws),
                payload_type: pt,
                rx: Arc::new(Mutex::new(rx)),
            };

            assert_eq!(lane.payload_type(), pt);
        }
    }

    // ===== DataLane clone tests =====

    #[wasm_bindgen_test]
    fn test_data_lane_clone_websocket() {
        use web_sys::WebSocket;

        let ws = WebSocket::new("ws://test.com").unwrap();
        let (_tx, rx) = mpsc::unbounded();

        let lane1 = DataLane::WebSocket {
            ws: Arc::new(ws),
            payload_type: PayloadType::RpcReliable,
            rx: Arc::new(Mutex::new(rx)),
        };

        let lane2 = lane1.clone();

        match (&lane1, &lane2) {
            (DataLane::WebSocket { ws: ws1, .. }, DataLane::WebSocket { ws: ws2, .. }) => {
                assert!(Arc::ptr_eq(ws1, ws2));
            }
            _ => panic!("Expected WebSocket lanes"),
        }
    }

    #[wasm_bindgen_test]
    fn test_data_lane_clone_postmessage() {
        use web_sys::MessageChannel;

        let channel = MessageChannel::new().unwrap();
        let port = channel.port1();
        let (_tx, rx) = mpsc::unbounded();

        let lane1 = DataLane::PostMessage {
            port: Arc::new(port),
            payload_type: PayloadType::MediaRtp,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: None,
        };

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

    // ===== DataLane debug tests =====

    #[wasm_bindgen_test]
    fn test_data_lane_debug_websocket() {
        use web_sys::WebSocket;

        let ws = WebSocket::new("ws://test.com").unwrap();
        let (_tx, rx) = mpsc::unbounded();

        let lane = DataLane::WebSocket {
            ws: Arc::new(ws),
            payload_type: PayloadType::RpcSignal,
            rx: Arc::new(Mutex::new(rx)),
        };

        let debug_str = format!("{:?}", lane);

        assert!(debug_str.contains("DataLane::WebSocket"));
        assert!(debug_str.contains("payload_type"));
        assert!(debug_str.contains("RpcSignal"));
    }

    #[wasm_bindgen_test]
    fn test_data_lane_debug_postmessage() {
        use web_sys::MessageChannel;

        let channel = MessageChannel::new().unwrap();
        let port = channel.port1();
        let (_tx, rx) = mpsc::unbounded();

        let lane = DataLane::PostMessage {
            port: Arc::new(port),
            payload_type: PayloadType::StreamLatencyFirst,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: None,
        };

        let debug_str = format!("{:?}", lane);

        assert!(debug_str.contains("DataLane::PostMessage"));
        assert!(debug_str.contains("payload_type"));
        assert!(debug_str.contains("StreamLatencyFirst"));
    }

    // ===== DataLane with PortFailureNotifier tests =====

    #[wasm_bindgen_test]
    fn test_data_lane_postmessage_with_notifier() {
        use web_sys::MessageChannel;

        let channel = MessageChannel::new().unwrap();
        let port = channel.port1();
        let (_tx, rx) = mpsc::unbounded();

        let wire_pool = Arc::new(WirePool::new());
        let notifier = PortFailureNotifier::new(wire_pool, ConnType::WebRTC);

        let lane = DataLane::PostMessage {
            port: Arc::new(port),
            payload_type: PayloadType::MediaRtp,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: Some(notifier),
        };

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
    fn test_data_lane_postmessage_without_notifier() {
        use web_sys::MessageChannel;

        let channel = MessageChannel::new().unwrap();
        let port = channel.port1();
        let (_tx, rx) = mpsc::unbounded();

        let lane = DataLane::PostMessage {
            port: Arc::new(port),
            payload_type: PayloadType::RpcReliable,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: None,
        };

        match lane {
            DataLane::PostMessage {
                failure_notifier, ..
            } => {
                assert!(failure_notifier.is_none());
            }
            _ => panic!("Expected PostMessage lane"),
        }
    }

    // ===== DataLane variant checks =====

    #[wasm_bindgen_test]
    fn test_data_lane_variant_websocket() {
        use web_sys::WebSocket;

        let ws = WebSocket::new("ws://test.com").unwrap();
        let (_tx, rx) = mpsc::unbounded();

        let lane = DataLane::WebSocket {
            ws: Arc::new(ws),
            payload_type: PayloadType::RpcReliable,
            rx: Arc::new(Mutex::new(rx)),
        };

        match lane {
            DataLane::WebSocket { .. } => {
                // Success
            }
            _ => panic!("Expected WebSocket variant"),
        }
    }

    #[wasm_bindgen_test]
    fn test_data_lane_variant_postmessage() {
        use web_sys::MessageChannel;

        let channel = MessageChannel::new().unwrap();
        let port = channel.port1();
        let (_tx, rx) = mpsc::unbounded();

        let lane = DataLane::PostMessage {
            port: Arc::new(port),
            payload_type: PayloadType::MediaRtp,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: None,
        };

        match lane {
            DataLane::PostMessage { .. } => {
                // Success
            }
            _ => panic!("Expected PostMessage variant"),
        }
    }

    // ===== Different PayloadType combination tests =====

    #[wasm_bindgen_test]
    fn test_different_payload_types_websocket() {
        use web_sys::WebSocket;

        let types = [
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
        ];

        for pt in types {
            let ws = WebSocket::new("ws://test.com").unwrap();
            let (_tx, rx) = mpsc::unbounded();

            let lane = DataLane::WebSocket {
                ws: Arc::new(ws),
                payload_type: pt,
                rx: Arc::new(Mutex::new(rx)),
            };

            assert_eq!(lane.payload_type(), pt);
        }
    }

    #[wasm_bindgen_test]
    fn test_different_payload_types_postmessage() {
        use web_sys::MessageChannel;

        let types = [
            PayloadType::MediaRtp,
            PayloadType::StreamLatencyFirst,
            PayloadType::RpcReliable,
        ];

        for pt in types {
            let channel = MessageChannel::new().unwrap();
            let port = channel.port1();
            let (_tx, rx) = mpsc::unbounded();

            let lane = DataLane::PostMessage {
                port: Arc::new(port),
                payload_type: pt,
                rx: Arc::new(Mutex::new(rx)),
                failure_notifier: None,
            };

            assert_eq!(lane.payload_type(), pt);
        }
    }
}
