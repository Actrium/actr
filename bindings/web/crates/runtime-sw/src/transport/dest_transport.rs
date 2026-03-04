//! DestTransport - 单个目标的传输层管理器
//!
//! 管理到特定 Dest 的所有连接和消息路由
//! 实现事件驱动模式，零轮询

use super::wire_handle::WireHandle;
use super::wire_pool::WirePool;
use actr_web_common::{ConnType, Dest, PayloadType, WebError, WebResult};
use bytes::Bytes;
use std::sync::Arc;

/// DestTransport - 单个目标的传输层管理器
///
/// 核心职责：
/// - 管理到特定 Dest 的所有连接（WebSocket + WebRTC）
/// - 后台并发建立连接（饱和连接模式）
/// - 事件驱动等待连接状态
/// - 缓存 WireHandle 内的 Lanes
/// - WirePool 处理优先级选择
pub struct DestTransport {
    /// 目标
    dest: Dest,

    /// 连接管理器
    wire_pool: Arc<WirePool>,
}

impl DestTransport {
    /// 创建新的 DestTransport
    ///
    /// # 参数
    /// - `dest`: 目标
    /// - `connections`: 预创建的连接列表（WebSocket/WebRTC），可为空
    pub async fn new(dest: Dest, connections: Vec<WireHandle>) -> WebResult<Self> {
        let wire_pool = Arc::new(WirePool::new());

        // 后台并发启动所有连接任务
        log::info!("[{:?}] Starting connection tasks...", dest);
        for conn in connections {
            wire_pool.add_connection(conn);
        }

        Ok(Self { dest, wire_pool })
    }

    /// 获取 WirePool 引用
    ///
    /// 用于在连接建立后从外部注入新的 WireHandle（例如 DOM 传入 MessagePort 后）。
    pub fn wire_pool(&self) -> &Arc<WirePool> {
        &self.wire_pool
    }

    /// 发送消息
    ///
    /// 核心设计：事件驱动等待
    /// - 如果连接可用，立即发送
    /// - 如果不可用，等待连接状态变更（通过 watch channel）
    /// - WirePool 已经处理优先级，只需要按顺序尝试 DataLane Types
    pub async fn send(&self, payload_type: PayloadType, data: &[u8]) -> WebResult<()> {
        log::debug!(
            "[{:?}] Sending message: type={:?}, size={}",
            self.dest,
            payload_type,
            data.len()
        );

        // 1. 确定需要的连接类型（简化版本：WebRTC 优先）
        let conn_types = self.get_conn_types_for(payload_type);

        if conn_types.is_empty() {
            return Err(WebError::Transport(format!(
                "No route for: {:?}",
                payload_type
            )));
        }

        // 2. 订阅连接状态变更
        let mut watcher = self.wire_pool.subscribe_changes();

        loop {
            // 3. 检查当前可用连接（快照）
            let ready_set = watcher.borrow_and_update();

            log::trace!("[{:?}] Available connections: {:?}", self.dest, ready_set);

            // 4. 按优先级尝试每种连接类型
            for &conn_type in &conn_types {
                // 检查此连接是否就绪
                if !ready_set.contains(&conn_type) {
                    log::trace!("[{:?}] {:?} not ready, trying next", self.dest, conn_type);
                    continue;
                }

                // 获取连接并创建/获取 Lane
                if let Some(conn) = self.wire_pool.get_connection(conn_type).await {
                    match conn.get_lane(payload_type).await {
                        Ok(lane) => {
                            log::debug!(
                                "[{:?}] Using connection: {:?} (type={:?})",
                                self.dest,
                                conn_type,
                                payload_type
                            );

                            // 转换为 Bytes（零拷贝）
                            return lane.send(Bytes::copy_from_slice(data)).await;
                        }
                        Err(e) => {
                            log::warn!(
                                "[{:?}] Failed to get DataLane: {:?}: {}",
                                self.dest,
                                conn_type,
                                e
                            );
                            continue;
                        }
                    }
                }
            }

            // 5. 所有尝试都失败，等待连接状态变更
            log::info!("[{:?}] Waiting for connection status...", self.dest);

            // 事件驱动等待！
            if watcher.changed().await.is_err() {
                return Err(WebError::Transport("connection manager closed".to_string()));
            }

            log::debug!("[{:?}] Connection status updated, retrying...", self.dest);
        }
    }

    /// 关闭 DestTransport 并释放所有连接资源
    pub async fn close(&self) -> WebResult<()> {
        log::info!("[{:?}] Closing DestTransport", self.dest);

        // 关闭所有连接
        for conn_type in [ConnType::WebSocket, ConnType::WebRTC] {
            if let Some(conn) = self.wire_pool.get_connection(conn_type).await {
                if let Err(e) = conn.close().await {
                    log::warn!(
                        "[{:?}] Failed to close {:?} connection: {}",
                        self.dest,
                        conn_type,
                        e
                    );
                } else {
                    log::debug!("[{:?}] Closed {:?} connection", self.dest, conn_type);
                }
            }
        }

        Ok(())
    }

    /// 检查是否有健康的连接
    ///
    /// 用于健康检查器检测失败的连接
    ///
    /// # 返回
    /// - `true`: 至少有一个连接健康（已连接）
    /// - `false`: 所有连接都不健康或不存在
    pub async fn has_healthy_connection(&self) -> bool {
        for conn_type in [ConnType::WebRTC, ConnType::WebSocket] {
            if let Some(conn) = self.wire_pool.get_connection(conn_type).await {
                if conn.is_connected() {
                    return true;
                }
            }
        }
        false
    }

    /// 获取给定 PayloadType 需要的连接类型（优先级排序）
    ///
    /// 简化版本：
    /// - MEDIA_RTP: 只能 WebRTC
    /// - 其他: WebRTC 优先，WebSocket fallback
    fn get_conn_types_for(&self, payload_type: PayloadType) -> Vec<ConnType> {
        match payload_type {
            PayloadType::MediaRtp => vec![ConnType::WebRTC],
            _ => vec![ConnType::WebRTC, ConnType::WebSocket],
        }
    }

    /// 获取目标
    pub fn dest(&self) -> &Dest {
        &self.dest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{WebRtcConnection, WebSocketConnection};
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    fn test_get_conn_types_for_media_rtp() {
        let dt = create_test_transport();
        let conn_types = dt.get_conn_types_for(PayloadType::MediaRtp);

        assert_eq!(conn_types.len(), 1);
        assert_eq!(conn_types[0], ConnType::WebRTC);
    }

    #[test]
    fn test_get_conn_types_for_rpc_reliable() {
        let dt = create_test_transport();
        let conn_types = dt.get_conn_types_for(PayloadType::RpcReliable);

        assert_eq!(conn_types.len(), 2);
        assert_eq!(conn_types[0], ConnType::WebRTC);
        assert_eq!(conn_types[1], ConnType::WebSocket);
    }

    #[test]
    fn test_get_conn_types_for_rpc_signal() {
        let dt = create_test_transport();
        let conn_types = dt.get_conn_types_for(PayloadType::RpcSignal);

        assert_eq!(conn_types.len(), 2);
        assert_eq!(conn_types[0], ConnType::WebRTC);
        assert_eq!(conn_types[1], ConnType::WebSocket);
    }

    #[test]
    fn test_get_conn_types_for_stream_reliable() {
        let dt = create_test_transport();
        let conn_types = dt.get_conn_types_for(PayloadType::StreamReliable);

        assert_eq!(conn_types.len(), 2);
        assert_eq!(conn_types[0], ConnType::WebRTC);
        assert_eq!(conn_types[1], ConnType::WebSocket);
    }

    #[wasm_bindgen_test]
    async fn test_dest_transport_new_server() {
        let dest = Dest::Server("wss://server.com".to_string());
        let ws = WebSocketConnection::new("wss://server.com");
        let handle = WireHandle::WebSocket(ws);

        let dt = DestTransport::new(dest.clone(), vec![handle]).await;
        assert!(dt.is_ok());

        let transport = dt.unwrap();
        assert_eq!(transport.dest(), &dest);
    }

    #[wasm_bindgen_test]
    async fn test_dest_transport_new_peer() {
        let dest = Dest::Peer("peer-123".to_string());
        let rtc = WebRtcConnection::new("peer-123".to_string());
        let handle = WireHandle::WebRTC(rtc);

        let dt = DestTransport::new(dest.clone(), vec![handle]).await;
        assert!(dt.is_ok());

        let transport = dt.unwrap();
        assert_eq!(transport.dest(), &dest);
    }

    #[wasm_bindgen_test]
    async fn test_dest_transport_new_empty_connections() {
        let dest = Dest::Server("wss://test.com".to_string());

        let dt = DestTransport::new(dest.clone(), vec![]).await;
        assert!(dt.is_ok());

        let transport = dt.unwrap();
        assert_eq!(transport.dest(), &dest);
    }

    #[wasm_bindgen_test]
    async fn test_dest_transport_new_multiple_connections() {
        let dest = Dest::Peer("peer-abc".to_string());

        let ws = WebSocketConnection::new("wss://fallback.com");
        let ws_handle = WireHandle::WebSocket(ws);

        let rtc = WebRtcConnection::new("peer-abc".to_string());
        let rtc_handle = WireHandle::WebRTC(rtc);

        let dt = DestTransport::new(dest.clone(), vec![ws_handle, rtc_handle]).await;
        assert!(dt.is_ok());

        let transport = dt.unwrap();
        assert_eq!(transport.dest(), &dest);
    }

    #[wasm_bindgen_test]
    async fn test_dest_transport_dest_accessor() {
        let dest1 = Dest::Server("wss://test1.com".to_string());
        let dest2 = Dest::Peer("peer-456".to_string());

        let dt1 = DestTransport::new(dest1.clone(), vec![]).await.unwrap();
        let dt2 = DestTransport::new(dest2.clone(), vec![]).await.unwrap();

        assert_eq!(dt1.dest(), &dest1);
        assert_eq!(dt2.dest(), &dest2);
        assert_ne!(dt1.dest(), dt2.dest());
    }

    #[wasm_bindgen_test]
    async fn test_has_healthy_connection_no_connections() {
        let dest = Dest::Server("wss://test.com".to_string());
        let dt = DestTransport::new(dest, vec![]).await.unwrap();

        let healthy = dt.has_healthy_connection().await;
        assert!(!healthy);
    }

    #[wasm_bindgen_test]
    async fn test_has_healthy_connection_with_connected() {
        let dest = Dest::Server("wss://test.com".to_string());
        let ws = WebSocketConnection::new("wss://test.com");

        // 先连接
        ws.connect().await.unwrap();

        let handle = WireHandle::WebSocket(ws);
        let dt = DestTransport::new(dest, vec![handle]).await.unwrap();

        // 需要等待连接状态更新
        gloo_timers::future::TimeoutFuture::new(100).await;

        let healthy = dt.has_healthy_connection().await;
        assert!(healthy);
    }

    #[test]
    fn test_conn_types_priority_order() {
        let dt = create_test_transport();

        // 非 RTP 类型应该 WebRTC 优先
        let types = dt.get_conn_types_for(PayloadType::RpcReliable);
        assert_eq!(types[0], ConnType::WebRTC);
        assert_eq!(types[1], ConnType::WebSocket);
    }

    #[test]
    fn test_media_rtp_only_webrtc() {
        let dt = create_test_transport();

        let types = dt.get_conn_types_for(PayloadType::MediaRtp);
        assert_eq!(types.len(), 1);
        assert!(!types.contains(&ConnType::WebSocket));
        assert!(types.contains(&ConnType::WebRTC));
    }

    #[test]
    fn test_dest_server_variant() {
        let dest = Dest::Server("wss://example.com".to_string());
        match dest {
            Dest::Server(url) => assert_eq!(url, "wss://example.com"),
            _ => panic!("Expected Server variant"),
        }
    }

    #[test]
    fn test_dest_peer_variant() {
        let dest = Dest::Peer("peer-xyz".to_string());
        match dest {
            Dest::Peer(id) => assert_eq!(id, "peer-xyz"),
            _ => panic!("Expected Peer variant"),
        }
    }

    // Helper function
    fn create_test_transport() -> DestTransport {
        let dest = Dest::Server("wss://test.com".to_string());
        let wire_pool = Arc::new(WirePool::new());
        DestTransport { dest, wire_pool }
    }
}
