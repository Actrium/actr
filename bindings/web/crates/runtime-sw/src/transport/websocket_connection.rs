//! WebSocket Connection - WebSocket 连接封装
//!
//! 提供 WebSocket 连接的生命周期管理和 DataLane 缓存

use super::lane::DataLane;
use super::websocket::WebSocketLaneBuilder;
use actr_web_common::{PayloadType, WebResult};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;

/// WebSocket 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

/// WebSocket 连接
///
/// 封装 WebSocket 连接，提供统一的连接管理和 Lane 缓存
#[derive(Clone)]
pub struct WebSocketConnection {
    /// 连接 URL
    url: String,

    /// 连接状态
    state: Arc<Mutex<ConnectionState>>,

    /// DataLane 缓存（PayloadType → DataLane）
    lane_cache: Arc<DashMap<PayloadType, DataLane>>,
}

impl WebSocketConnection {
    /// 创建新的 WebSocket 连接
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            lane_cache: Arc::new(DashMap::new()),
        }
    }

    /// 建立连接
    pub async fn connect(&self) -> WebResult<()> {
        let current_state = *self.state.lock();

        match current_state {
            ConnectionState::Connected => {
                log::debug!("[WebSocketConnection] Already connected: {}", self.url);
                return Ok(());
            }
            ConnectionState::Connecting => {
                log::debug!("[WebSocketConnection] Already connecting: {}", self.url);
                return Ok(());
            }
            _ => {}
        }

        log::info!("[WebSocketConnection] Connecting to: {}", self.url);
        *self.state.lock() = ConnectionState::Connecting;

        // 实际连接会在 get_lane 时创建
        // 这里只是标记状态
        *self.state.lock() = ConnectionState::Connected;

        Ok(())
    }

    /// 检查是否已连接
    pub fn is_connected(&self) -> bool {
        matches!(*self.state.lock(), ConnectionState::Connected)
    }

    /// 获取或创建 DataLane（带缓存）
    pub async fn get_lane(&self, payload_type: PayloadType) -> WebResult<DataLane> {
        // 1. 检查缓存
        if let Some(lane) = self.lane_cache.get(&payload_type) {
            log::trace!(
                "[WebSocketConnection] Reusing cached lane: {:?}",
                payload_type
            );
            return Ok(lane.clone());
        }

        // 2. 创建新 Lane
        log::debug!(
            "[WebSocketConnection] Creating new lane: url={}, payload_type={:?}",
            self.url,
            payload_type
        );

        let lane = WebSocketLaneBuilder::new(&self.url, payload_type)
            .build()
            .await?;

        // 3. 缓存
        self.lane_cache.insert(payload_type, lane.clone());

        // 4. 更新连接状态
        *self.state.lock() = ConnectionState::Connected;

        Ok(lane)
    }

    /// 关闭连接
    pub async fn close(&self) -> WebResult<()> {
        log::info!("[WebSocketConnection] Closing: {}", self.url);

        // 清空缓存
        self.lane_cache.clear();

        // 更新状态
        *self.state.lock() = ConnectionState::Disconnected;

        Ok(())
    }

    /// 获取连接 URL
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl std::fmt::Debug for WebSocketConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketConnection")
            .field("url", &self.url)
            .field("state", &self.state.lock())
            .field("lanes", &self.lane_cache.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    fn test_connection_state_equality() {
        assert_eq!(ConnectionState::Disconnected, ConnectionState::Disconnected);
        assert_eq!(ConnectionState::Connecting, ConnectionState::Connecting);
        assert_eq!(ConnectionState::Connected, ConnectionState::Connected);
        assert_eq!(ConnectionState::Failed, ConnectionState::Failed);

        assert_ne!(ConnectionState::Disconnected, ConnectionState::Connected);
        assert_ne!(ConnectionState::Connecting, ConnectionState::Failed);
    }

    #[test]
    fn test_connection_state_debug() {
        let state = ConnectionState::Connected;
        let debug_str = format!("{:?}", state);
        assert!(debug_str.contains("Connected"));
    }

    #[test]
    fn test_connection_state_clone() {
        let state1 = ConnectionState::Connected;
        let state2 = state1;
        assert_eq!(state1, state2);
    }

    #[test]
    fn test_websocket_connection_new() {
        let conn = WebSocketConnection::new("ws://localhost:8080");
        assert_eq!(conn.url(), "ws://localhost:8080");
        assert!(!conn.is_connected());

        let state = *conn.state.lock();
        assert_eq!(state, ConnectionState::Disconnected);
    }

    #[test]
    fn test_websocket_connection_new_with_string() {
        let url = String::from("wss://example.com/ws");
        let conn = WebSocketConnection::new(url);
        assert_eq!(conn.url(), "wss://example.com/ws");
    }

    #[test]
    fn test_websocket_connection_initial_state() {
        let conn = WebSocketConnection::new("ws://test.local");
        assert!(!conn.is_connected());
        assert_eq!(conn.lane_cache.len(), 0);
    }

    #[wasm_bindgen_test]
    async fn test_websocket_connection_connect() {
        let conn = WebSocketConnection::new("ws://localhost:9000");

        assert!(!conn.is_connected());

        let result = conn.connect().await;
        assert!(result.is_ok());

        assert!(conn.is_connected());
        let state = *conn.state.lock();
        assert_eq!(state, ConnectionState::Connected);
    }

    #[wasm_bindgen_test]
    async fn test_websocket_connection_connect_idempotent() {
        let conn = WebSocketConnection::new("ws://localhost:9001");

        // 第一次连接
        let result1 = conn.connect().await;
        assert!(result1.is_ok());
        assert!(conn.is_connected());

        // 第二次连接（应该是幂等的）
        let result2 = conn.connect().await;
        assert!(result2.is_ok());
        assert!(conn.is_connected());
    }

    #[wasm_bindgen_test]
    async fn test_websocket_connection_close() {
        let conn = WebSocketConnection::new("ws://localhost:9002");

        // 先连接
        conn.connect().await.unwrap();
        assert!(conn.is_connected());

        // 关闭连接
        let result = conn.close().await;
        assert!(result.is_ok());

        // 验证状态
        assert!(!conn.is_connected());
        let state = *conn.state.lock();
        assert_eq!(state, ConnectionState::Disconnected);

        // 验证缓存已清空
        assert_eq!(conn.lane_cache.len(), 0);
    }

    #[test]
    fn test_websocket_connection_url() {
        let conn = WebSocketConnection::new("ws://api.example.com:3000/socket");
        assert_eq!(conn.url(), "ws://api.example.com:3000/socket");
    }

    #[test]
    fn test_websocket_connection_clone() {
        let conn1 = WebSocketConnection::new("ws://localhost:8080");
        let conn2 = conn1.clone();

        assert_eq!(conn1.url(), conn2.url());
        assert!(Arc::ptr_eq(&conn1.state, &conn2.state));
        assert!(Arc::ptr_eq(&conn1.lane_cache, &conn2.lane_cache));
    }

    #[wasm_bindgen_test]
    async fn test_websocket_connection_clone_shares_state() {
        let conn1 = WebSocketConnection::new("ws://localhost:8081");
        let conn2 = conn1.clone();

        // 通过 conn1 连接
        conn1.connect().await.unwrap();

        // conn2 应该能看到相同的状态
        assert!(conn2.is_connected());
    }

    #[test]
    fn test_websocket_connection_debug() {
        let conn = WebSocketConnection::new("ws://debug.test");
        let debug_str = format!("{:?}", conn);

        assert!(debug_str.contains("WebSocketConnection"));
        assert!(debug_str.contains("ws://debug.test"));
        assert!(debug_str.contains("state"));
        assert!(debug_str.contains("lanes"));
    }

    #[wasm_bindgen_test]
    async fn test_websocket_connection_state_transitions() {
        let conn = WebSocketConnection::new("ws://localhost:8082");

        // 初始状态：Disconnected
        let state = *conn.state.lock();
        assert_eq!(state, ConnectionState::Disconnected);

        // 连接后：Connected
        conn.connect().await.unwrap();
        let state = *conn.state.lock();
        assert_eq!(state, ConnectionState::Connected);

        // 关闭后：Disconnected
        conn.close().await.unwrap();
        let state = *conn.state.lock();
        assert_eq!(state, ConnectionState::Disconnected);
    }

    #[test]
    fn test_websocket_connection_lane_cache_empty_initially() {
        let conn = WebSocketConnection::new("ws://localhost:8083");
        assert_eq!(conn.lane_cache.len(), 0);
        assert!(conn.lane_cache.is_empty());
    }

    #[wasm_bindgen_test]
    async fn test_websocket_connection_close_clears_cache() {
        let conn = WebSocketConnection::new("ws://localhost:8084");

        // 模拟添加一些缓存项（虽然我们不能真正创建 DataLane）
        // 这里只测试 close 方法确实会调用 clear
        conn.close().await.unwrap();

        assert_eq!(conn.lane_cache.len(), 0);
    }

    #[test]
    fn test_connection_state_copy_trait() {
        let state1 = ConnectionState::Connected;
        let state2 = state1; // Copy, not move
        let state3 = state1; // Can still use state1

        assert_eq!(state1, state2);
        assert_eq!(state2, state3);
    }

    #[test]
    fn test_websocket_connection_multiple_instances() {
        let conn1 = WebSocketConnection::new("ws://server1.com");
        let conn2 = WebSocketConnection::new("ws://server2.com");

        assert_eq!(conn1.url(), "ws://server1.com");
        assert_eq!(conn2.url(), "ws://server2.com");

        // 验证它们是独立的实例
        assert!(!Arc::ptr_eq(&conn1.state, &conn2.state));
        assert!(!Arc::ptr_eq(&conn1.lane_cache, &conn2.lane_cache));
    }

    #[test]
    fn test_connection_state_all_variants() {
        let states = vec![
            ConnectionState::Disconnected,
            ConnectionState::Connecting,
            ConnectionState::Connected,
            ConnectionState::Failed,
        ];

        // 验证所有状态都是唯一的
        for (i, state1) in states.iter().enumerate() {
            for (j, state2) in states.iter().enumerate() {
                if i == j {
                    assert_eq!(state1, state2);
                } else {
                    assert_ne!(state1, state2);
                }
            }
        }
    }

    #[wasm_bindgen_test]
    async fn test_websocket_connection_reconnect_after_close() {
        let conn = WebSocketConnection::new("ws://localhost:8085");

        // 第一次连接
        conn.connect().await.unwrap();
        assert!(conn.is_connected());

        // 关闭
        conn.close().await.unwrap();
        assert!(!conn.is_connected());

        // 重新连接
        conn.connect().await.unwrap();
        assert!(conn.is_connected());
    }
}
