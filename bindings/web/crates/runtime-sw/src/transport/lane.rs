//! DataLane - Service Worker 环境的数据传输通道
//!
//! Service Worker 端只包含：
//! - WebSocket Lane：与服务器通信
//! - PostMessage Lane：与 DOM 通信

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

/// Service Worker 环境传输层操作结果
pub type LaneResult<T> = WebResult<T>;

/// Port 失效通知器
///
/// 当 MessagePort 发送失败时，通知 WirePool 标记连接为失效
#[derive(Clone)]
pub struct PortFailureNotifier {
    wire_pool: Arc<WirePool>,
    conn_type: ConnType,
}

impl PortFailureNotifier {
    /// 创建新的失效通知器
    pub fn new(wire_pool: Arc<WirePool>, conn_type: ConnType) -> Self {
        Self {
            wire_pool,
            conn_type,
        }
    }

    /// 通知 Port 失效
    pub fn notify_port_failed(&self) {
        log::warn!(
            "[PortFailureNotifier] Notifying port failure: {:?}",
            self.conn_type
        );

        self.wire_pool.mark_connection_failed(self.conn_type);
    }
}

/// DataLane - Service Worker 环境的数据传输通道
#[derive(Clone)]
pub enum DataLane {
    /// WebSocket Lane
    ///
    /// 与服务器的 WebSocket 连接
    /// 支持：RPC_*, STREAM_* (不支持 MEDIA_RTP)
    WebSocket {
        ws: Arc<WebSocket>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },

    /// PostMessage Lane
    ///
    /// 与 DOM 的通信通道
    /// 支持：全部 PayloadType
    PostMessage {
        port: Arc<MessagePort>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
        failure_notifier: Option<PortFailureNotifier>,
    },
}

impl DataLane {
    /// 发送消息（零拷贝优化）
    pub async fn send(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::WebSocket {
                ws, payload_type, ..
            } => {
                // ✅ 构造消息：使用零拷贝辅助函数
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // ✅ 直接发送 Bytes slice（WebSocket API 接受 &[u8]）
                ws.send_with_u8_array(&msg)
                    .map_err(|e| WebError::Transport(format!("WebSocket send failed: {:?}", e)))?;

                log::trace!(
                    "WebSocket Lane 发送消息: payload_type={:?}, size={} bytes",
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
                // ✅ 构造消息：使用零拷贝辅助函数
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // ✅ 零拷贝发送：创建 WASM 内存视图
                let js_view = send_zero_copy(&msg);

                // 尝试发送，捕获失败
                match port.post_message(&js_view.into()) {
                    Ok(_) => {
                        log::trace!(
                            "PostMessage Lane 发送消息: payload_type={:?}, size={} bytes",
                            payload_type,
                            data.len()
                        );
                        Ok(())
                    }
                    Err(e) => {
                        // MessagePort 失效了
                        log::error!("PostMessage send failed (port may be dead): {:?}", e);

                        // 通知 WirePool 连接失效
                        if let Some(notifier) = failure_notifier {
                            notifier.notify_port_failed();
                        }

                        Err(WebError::Transport(format!("PostMessage failed: {:?}", e)))
                    }
                }
            }
        }
    }

    /// 使用 Transferable Objects 发送消息（仅 PostMessage）
    ///
    /// **Transferable Objects**：
    /// - 转移 ArrayBuffer 所有权而非拷贝
    /// - 适用于大数据传输（>10KB）
    /// - 仅支持 PostMessage Lane（WebSocket 不支持）
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
                port,
                payload_type,
                failure_notifier,
                ..
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
                            "PostMessage Lane (SW) 使用 transfer 发送消息: payload_type={:?}, size={} bytes",
                            payload_type,
                            data.len()
                        );
                        Ok(())
                    }
                    Err(e) => {
                        // MessagePort 失效了
                        log::error!(
                            "PostMessage with transfer failed (port may be dead): {:?}",
                            e
                        );

                        // 通知 WirePool 连接失效
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
                // WebSocket 不支持 Transferable Objects，回退到普通 send
                log::warn!("WebSocket 不支持 Transferable Objects，回退到普通 send");
                self.send(data).await
            }
        }
    }

    /// 自动选择发送方式（根据数据大小）
    ///
    /// **决策逻辑**：
    /// - PostMessage + 数据 >= 10KB → 使用 `send_with_transfer()`
    /// - PostMessage + 数据 < 10KB → 使用普通 `send()`
    /// - WebSocket → 使用普通 `send()`
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
            DataLane::WebSocket {
                rx, payload_type, ..
            } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "WebSocket Lane 接收消息: payload_type={:?}, size={} bytes",
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
                    "PostMessage Lane 接收消息: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );
                Some(data)
            }
        }
    }

    /// 获取 PayloadType
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

    // ===== PortFailureNotifier 测试 =====

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

    // ===== DataLane PayloadType 测试 =====

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

    // ===== DataLane Clone 测试 =====

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

    // ===== DataLane Debug 测试 =====

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

    // ===== DataLane with PortFailureNotifier 测试 =====

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

    // ===== DataLane 变体类型检查 =====

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

    // ===== 不同 PayloadType 组合测试 =====

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
