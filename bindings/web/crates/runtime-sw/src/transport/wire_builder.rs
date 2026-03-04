//! Wire Builder - Wire 层连接工厂
//!
//! 提供工厂模式创建 Wire 层连接（WebSocket/WebRTC）

use super::websocket_connection::WebSocketConnection;
use super::wire_handle::WireHandle;
use actr_web_common::{ControlMessage, CreateP2PRequest, Dest, WebResult};
use async_trait::async_trait;
use std::sync::Arc;

/// Wire 构建器 trait：异步创建基于 Dest 的 Wire 组件
///
/// 实现此 trait 自定义 Wire 层组件创建逻辑（例如 WebRTC, WebSocket）
#[async_trait(?Send)] // WASM 环境不支持 Send
pub trait WireBuilder {
    /// 创建到指定 Dest 的连接
    ///
    /// # 参数
    /// - `dest`: 目标地址
    ///
    /// # 返回
    /// - Wire 句柄列表（可能包含多种类型：WebSocket, WebRTC 等）
    async fn create_connections(&self, dest: &Dest) -> WebResult<Vec<WireHandle>>;
}

/// Web 环境的 WireBuilder 实现
pub struct WebWireBuilder {
    /// DOM 通道（用于请求 P2P 创建）
    dom_channel: Arc<parking_lot::Mutex<Option<super::lane::DataLane>>>,

    /// 请求 ID 计数器
    request_counter: Arc<parking_lot::Mutex<u64>>,
}

impl WebWireBuilder {
    /// 创建新的 WebWireBuilder
    pub fn new() -> Self {
        Self {
            dom_channel: Arc::new(parking_lot::Mutex::new(None)),
            request_counter: Arc::new(parking_lot::Mutex::new(0)),
        }
    }

    /// 设置 DOM 通道
    pub fn set_dom_channel(&self, channel: super::lane::DataLane) {
        let mut dom = self.dom_channel.lock();
        *dom = Some(channel);
        log::info!("[WebWireBuilder] DOM channel set");
    }

    /// 生成请求 ID
    fn next_request_id(&self) -> String {
        let mut counter = self.request_counter.lock();
        *counter += 1;
        format!("p2p-{}", *counter)
    }

    /// 请求 DOM 创建 P2P 连接（异步，不阻塞）
    fn request_p2p_creation(&self, dest: Dest) {
        let dom_channel = self.dom_channel.clone();
        let request_id = self.next_request_id();

        wasm_bindgen_futures::spawn_local(async move {
            let channel = dom_channel.lock();
            if let Some(lane) = channel.as_ref() {
                // 创建请求
                let request = CreateP2PRequest::new(dest.clone(), request_id.clone());
                let control_msg = ControlMessage::CreateP2P(request);

                // 序列化并发送
                match control_msg.serialize() {
                    Ok(data) => {
                        if let Err(e) = lane.send(data).await {
                            log::error!("[WebWireBuilder] Failed to send P2P request: {}", e);
                        } else {
                            log::debug!(
                                "[WebWireBuilder] Requested P2P creation: {:?} (id={})",
                                dest,
                                request_id
                            );
                        }
                    }
                    Err(e) => {
                        log::error!("[WebWireBuilder] Failed to serialize P2P request: {}", e);
                    }
                }
            } else {
                log::debug!("[WebWireBuilder] DOM channel not available, cannot request P2P");
            }
        });
    }
}

impl Default for WebWireBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl WireBuilder for WebWireBuilder {
    async fn create_connections(&self, dest: &Dest) -> WebResult<Vec<WireHandle>> {
        let mut connections = Vec::new();

        match dest {
            Dest::Server(url) => {
                // 直接创建 WebSocket
                log::debug!("[WebWireBuilder] Creating WebSocket to: {}", url);
                let ws = WebSocketConnection::new(url);
                connections.push(WireHandle::WebSocket(ws));
            }

            Dest::Peer(peer_id) => {
                // Peer 连接仅通过 WebRTC DataChannel（由 DOM 侧 WebRTC Coordinator 建立）
                // 不创建 WebSocket fallback — 真实的中继通过 Signaling Server 完成

                // 异步请求 DOM 创建 P2P（不等待）
                log::debug!("[WebWireBuilder] Requesting P2P creation: {}", peer_id);
                self.request_p2p_creation(dest.clone());

                // P2P 创建完成后会通过 inject_connection 注入 WireHandle
            }
        }

        Ok(connections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    fn test_web_wire_builder_new() {
        let builder = WebWireBuilder::new();

        let counter = builder.request_counter.lock();
        assert_eq!(*counter, 0);

        let dom_channel = builder.dom_channel.lock();
        assert!(dom_channel.is_none());
    }

    #[test]
    fn test_web_wire_builder_default() {
        let builder = WebWireBuilder::default();

        let counter = builder.request_counter.lock();
        assert_eq!(*counter, 0);
    }

    #[test]
    fn test_next_request_id() {
        let builder = WebWireBuilder::new();

        let id1 = builder.next_request_id();
        assert_eq!(id1, "p2p-1");

        let id2 = builder.next_request_id();
        assert_eq!(id2, "p2p-2");

        let id3 = builder.next_request_id();
        assert_eq!(id3, "p2p-3");
    }

    #[test]
    fn test_next_request_id_increment() {
        let builder = WebWireBuilder::new();

        for i in 1..=10 {
            let id = builder.next_request_id();
            let expected = format!("p2p-{}", i);
            assert_eq!(id, expected);
        }
    }

    #[test]
    fn test_request_counter_starts_at_zero() {
        let builder = WebWireBuilder::new();

        let counter = builder.request_counter.lock();
        assert_eq!(*counter, 0);
    }

    #[test]
    fn test_dom_channel_initially_none() {
        let builder = WebWireBuilder::new();

        let dom_channel = builder.dom_channel.lock();
        assert!(dom_channel.is_none());
    }

    #[wasm_bindgen_test]
    async fn test_create_connections_server() {
        let builder = WebWireBuilder::new();
        let dest = Dest::Server("wss://server.example.com".to_string());

        let result = builder.create_connections(&dest).await;
        assert!(result.is_ok());

        let connections = result.unwrap();
        assert_eq!(connections.len(), 1);

        // 验证是 WebSocket 类型
        match &connections[0] {
            WireHandle::WebSocket(ws) => {
                assert_eq!(ws.url(), "wss://server.example.com");
            }
            _ => panic!("Expected WebSocket connection"),
        }
    }

    #[wasm_bindgen_test]
    async fn test_create_connections_peer() {
        let builder = WebWireBuilder::new();
        let dest = Dest::Peer("peer-123".to_string());

        let result = builder.create_connections(&dest).await;
        assert!(result.is_ok());

        let connections = result.unwrap();
        assert_eq!(connections.len(), 1);

        // 验证创建了 WebSocket fallback
        match &connections[0] {
            WireHandle::WebSocket(ws) => {
                assert_eq!(ws.url(), "wss://relay.example.com/peer/peer-123");
            }
            _ => panic!("Expected WebSocket connection"),
        }
    }

    #[wasm_bindgen_test]
    async fn test_create_connections_multiple_dests() {
        let builder = WebWireBuilder::new();

        let dest1 = Dest::Server("wss://server1.com".to_string());
        let result1 = builder.create_connections(&dest1).await;
        assert!(result1.is_ok());
        assert_eq!(result1.unwrap().len(), 1);

        let dest2 = Dest::Peer("peer-456".to_string());
        let result2 = builder.create_connections(&dest2).await;
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap().len(), 1);
    }

    #[test]
    fn test_builder_clone_shares_state() {
        let builder = WebWireBuilder::new();

        let dom_clone = builder.dom_channel.clone();
        let counter_clone = builder.request_counter.clone();

        // 验证 Arc 共享
        assert!(Arc::ptr_eq(&builder.dom_channel, &dom_clone));
        assert!(Arc::ptr_eq(&builder.request_counter, &counter_clone));
    }

    #[test]
    fn test_next_request_id_format() {
        let builder = WebWireBuilder::new();

        let id = builder.next_request_id();
        assert!(id.starts_with("p2p-"));
        assert!(id.len() > 4);
    }

    #[test]
    fn test_multiple_builders_independent() {
        let builder1 = WebWireBuilder::new();
        let builder2 = WebWireBuilder::new();

        // 验证它们有独立的计数器
        let id1 = builder1.next_request_id();
        let id2 = builder2.next_request_id();

        assert_eq!(id1, "p2p-1");
        assert_eq!(id2, "p2p-1");

        // 验证 Arc 不共享
        assert!(!Arc::ptr_eq(
            &builder1.request_counter,
            &builder2.request_counter
        ));
    }

    #[test]
    fn test_request_counter_large_numbers() {
        let builder = WebWireBuilder::new();

        // 设置一个大的初始值
        {
            let mut counter = builder.request_counter.lock();
            *counter = 999;
        }

        let id = builder.next_request_id();
        assert_eq!(id, "p2p-1000");
    }
}
