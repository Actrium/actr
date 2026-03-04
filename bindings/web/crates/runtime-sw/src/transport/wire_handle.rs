//! Wire Handle - 统一的 Wire 层组件句柄
//!
//! 为 Wire 层组件（WebSocket/WebRTC）提供统一的访问接口

use super::lane::DataLane;
use super::websocket_connection::WebSocketConnection;
use actr_web_common::{ConnType, PayloadType, WebError, WebResult};
use dashmap::DashMap;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;

/// WebRTC 连接（通过 DOM 创建）
///
/// SW 端持有的是 MessagePort 引用，实际 PeerConnection 在 DOM
#[derive(Clone)]
pub struct WebRtcConnection {
    /// 对等端 ID
    peer_id: String,

    /// DataChannel 的 MessagePort（从 DOM 转移过来）
    /// 当 DOM 创建好 P2P 后，会通过 transferable 将 MessagePort 发送给 SW
    datachannel_port: Option<Arc<web_sys::MessagePort>>,

    /// 连接状态
    connected: Arc<parking_lot::Mutex<bool>>,

    /// DataLane 缓存（按 PayloadType 缓存，避免重复创建）
    lane_cache: Arc<DashMap<PayloadType, DataLane>>,
}

impl WebRtcConnection {
    /// 创建新的 WebRTC 连接句柄（初始未连接）
    pub fn new(peer_id: String) -> Self {
        Self {
            peer_id,
            datachannel_port: None,
            connected: Arc::new(parking_lot::Mutex::new(false)),
            lane_cache: Arc::new(DashMap::new()),
        }
    }

    /// 设置 DataChannel MessagePort（DOM 创建完成后调用）
    pub fn set_datachannel_port(&mut self, port: web_sys::MessagePort) {
        self.datachannel_port = Some(Arc::new(port));
        *self.connected.lock() = true;
        log::info!(
            "[WebRtcConnection] DataChannel port set for: {}",
            self.peer_id
        );
    }

    /// 建立连接（WebRTC 实际由 DOM 创建，这里只是等待）
    pub async fn connect(&self) -> WebResult<()> {
        // WebRTC 连接由 DOM 创建，SW 这边不主动连接
        // 如果已经有 port，说明已连接
        if self.datachannel_port.is_some() {
            Ok(())
        } else {
            Err(WebError::Transport(
                "WebRTC connection not ready (waiting for DOM)".to_string(),
            ))
        }
    }

    /// 检查是否已连接
    pub fn is_connected(&self) -> bool {
        *self.connected.lock()
    }

    /// 获取或创建 DataLane（带缓存）
    ///
    /// SW 端的 WebRTC 通过 MessagePort 桥接到 DOM：
    ///   SW DataLane::PostMessage → MessagePort → DOM → RtcDataChannel
    ///
    /// 缓存策略与 WebSocketConnection::get_lane 一致。
    pub async fn get_lane(&self, payload_type: PayloadType) -> WebResult<DataLane> {
        // 1. 检查缓存
        if let Some(lane) = self.lane_cache.get(&payload_type) {
            log::trace!(
                "[WebRtcConnection] Reusing cached lane: peer={} type={:?}",
                self.peer_id,
                payload_type
            );
            return Ok(lane.clone());
        }

        // 2. 需要 datachannel_port 才能创建 Lane
        let port = self.datachannel_port.as_ref().ok_or_else(|| {
            WebError::Transport("WebRTC connection not ready (no MessagePort)".to_string())
        })?;

        log::debug!(
            "[WebRtcConnection] Creating PostMessage lane: peer={} type={:?}",
            self.peer_id,
            payload_type
        );

        // 3. 直接构造 DataLane::PostMessage
        //
        // 发送方向：SW → datachannel_port.postMessage() → DOM bridge → RtcDataChannel.send()
        // 接收方向：DOM bridge → SW handle_fast_path（不经过此 lane 的 rx）
        //
        // rx 通道保留为空（接收走 handle_fast_path），仅用于发送。
        let (_tx, rx) = mpsc::unbounded();
        let lane = DataLane::PostMessage {
            port: Arc::clone(port),
            payload_type,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: None,
        };

        // 4. 缓存
        self.lane_cache.insert(payload_type, lane.clone());

        log::info!(
            "[WebRtcConnection] DataLane created: peer={} type={:?}",
            self.peer_id,
            payload_type
        );

        Ok(lane)
    }

    /// 关闭连接
    pub async fn close(&self) -> WebResult<()> {
        *self.connected.lock() = false;
        self.lane_cache.clear();
        log::info!("[WebRtcConnection] Closed: {}", self.peer_id);
        Ok(())
    }

    /// 获取对等端 ID
    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }
}

impl std::fmt::Debug for WebRtcConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebRtcConnection")
            .field("peer_id", &self.peer_id)
            .field("connected", &self.is_connected())
            .finish()
    }
}

/// WireHandle - Wire 层组件的统一句柄
///
/// # 设计理念
/// - 使用枚举 dispatch 而非 trait object，实现零虚拟调用开销
/// - 提供统一接口访问不同的 Wire 层实现
/// - 支持连接优先级比较（WebRTC > WebSocket）
#[derive(Clone, Debug)]
pub enum WireHandle {
    /// WebSocket 连接句柄
    WebSocket(WebSocketConnection),

    /// WebRTC 连接句柄
    WebRTC(WebRtcConnection),
}

impl WireHandle {
    /// 获取连接类型
    #[inline]
    pub fn conn_type(&self) -> ConnType {
        match self {
            WireHandle::WebSocket(_) => ConnType::WebSocket,
            WireHandle::WebRTC(_) => ConnType::WebRTC,
        }
    }

    /// 获取连接类型名称
    #[inline]
    pub fn connection_type(&self) -> &'static str {
        match self {
            WireHandle::WebSocket(_) => "WebSocket",
            WireHandle::WebRTC(_) => "WebRTC",
        }
    }

    /// 连接优先级（数值越高优先级越高）
    #[inline]
    pub fn priority(&self) -> u8 {
        match self {
            WireHandle::WebSocket(_) => 0,
            WireHandle::WebRTC(_) => 1, // WebRTC 优先级更高
        }
    }

    /// 建立连接
    #[inline]
    pub async fn connect(&self) -> WebResult<()> {
        match self {
            WireHandle::WebSocket(ws) => ws.connect().await,
            WireHandle::WebRTC(rtc) => rtc.connect().await,
        }
    }

    /// 检查是否已连接
    #[inline]
    pub fn is_connected(&self) -> bool {
        match self {
            WireHandle::WebSocket(ws) => ws.is_connected(),
            WireHandle::WebRTC(rtc) => rtc.is_connected(),
        }
    }

    /// 关闭连接
    #[inline]
    pub async fn close(&self) -> WebResult<()> {
        match self {
            WireHandle::WebSocket(ws) => ws.close().await,
            WireHandle::WebRTC(rtc) => rtc.close().await,
        }
    }

    /// 获取或创建 DataLane（带缓存）
    #[inline]
    pub async fn get_lane(&self, payload_type: PayloadType) -> WebResult<DataLane> {
        match self {
            WireHandle::WebSocket(ws) => ws.get_lane(payload_type).await,
            WireHandle::WebRTC(rtc) => rtc.get_lane(payload_type).await,
        }
    }

    /// 转换为 WebRTC 连接（如果是的话）
    #[inline]
    pub fn as_webrtc(&self) -> Option<&WebRtcConnection> {
        match self {
            WireHandle::WebRTC(rtc) => Some(rtc),
            _ => None,
        }
    }

    /// 转换为 WebRTC 连接（可变）
    #[inline]
    pub fn as_webrtc_mut(&mut self) -> Option<&mut WebRtcConnection> {
        match self {
            WireHandle::WebRTC(rtc) => Some(rtc),
            _ => None,
        }
    }

    /// 转换为 WebSocket 连接（如果是的话）
    #[inline]
    pub fn as_websocket(&self) -> Option<&WebSocketConnection> {
        match self {
            WireHandle::WebSocket(ws) => Some(ws),
            _ => None,
        }
    }
}

/// Wire 连接状态
#[derive(Debug, Clone)]
pub enum WireStatus {
    /// 连接中
    Connecting,

    /// 连接就绪
    Ready(WireHandle),

    /// 连接失败
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // ===== WebRtcConnection 测试 =====

    #[test]
    fn test_webrtc_connection_new() {
        let conn = WebRtcConnection::new("peer-abc".to_string());
        assert_eq!(conn.peer_id(), "peer-abc");
        assert!(!conn.is_connected());
        assert!(conn.datachannel_port.is_none());
    }

    #[test]
    fn test_webrtc_connection_peer_id() {
        let conn = WebRtcConnection::new("peer-123".to_string());
        assert_eq!(conn.peer_id(), "peer-123");
    }

    #[test]
    fn test_webrtc_connection_initial_state() {
        let conn = WebRtcConnection::new("peer-test".to_string());
        assert!(!conn.is_connected());

        let connected = conn.connected.lock();
        assert!(!*connected);
    }

    #[wasm_bindgen_test]
    async fn test_webrtc_connection_connect_before_port_set() {
        let conn = WebRtcConnection::new("peer-456".to_string());

        let result = conn.connect().await;
        assert!(result.is_err());

        if let Err(e) = result {
            match e {
                WebError::Transport(msg) => {
                    assert!(msg.contains("not ready"));
                }
                _ => panic!("Expected Transport error"),
            }
        }
    }

    #[wasm_bindgen_test]
    async fn test_webrtc_connection_close() {
        let conn = WebRtcConnection::new("peer-789".to_string());

        let result = conn.close().await;
        assert!(result.is_ok());
        assert!(!conn.is_connected());
    }

    #[test]
    fn test_webrtc_connection_clone() {
        let conn1 = WebRtcConnection::new("peer-clone".to_string());
        let conn2 = conn1.clone();

        assert_eq!(conn1.peer_id(), conn2.peer_id());
        assert!(Arc::ptr_eq(&conn1.connected, &conn2.connected));
    }

    #[test]
    fn test_webrtc_connection_debug() {
        let conn = WebRtcConnection::new("peer-debug".to_string());
        let debug_str = format!("{:?}", conn);

        assert!(debug_str.contains("WebRtcConnection"));
        assert!(debug_str.contains("peer-debug"));
        assert!(debug_str.contains("connected"));
    }

    // ===== WireHandle 测试 =====

    #[test]
    fn test_wire_handle_websocket_conn_type() {
        let ws = WebSocketConnection::new("ws://test.com");
        let handle = WireHandle::WebSocket(ws);

        assert_eq!(handle.conn_type(), ConnType::WebSocket);
        assert_eq!(handle.connection_type(), "WebSocket");
    }

    #[test]
    fn test_wire_handle_webrtc_conn_type() {
        let rtc = WebRtcConnection::new("peer-test".to_string());
        let handle = WireHandle::WebRTC(rtc);

        assert_eq!(handle.conn_type(), ConnType::WebRTC);
        assert_eq!(handle.connection_type(), "WebRTC");
    }

    #[test]
    fn test_wire_handle_priority() {
        let ws = WebSocketConnection::new("ws://test.com");
        let ws_handle = WireHandle::WebSocket(ws);

        let rtc = WebRtcConnection::new("peer-test".to_string());
        let rtc_handle = WireHandle::WebRTC(rtc);

        // WebRTC 优先级更高
        assert_eq!(ws_handle.priority(), 0);
        assert_eq!(rtc_handle.priority(), 1);
        assert!(rtc_handle.priority() > ws_handle.priority());
    }

    #[test]
    fn test_wire_handle_as_webrtc() {
        let rtc = WebRtcConnection::new("peer-123".to_string());
        let handle = WireHandle::WebRTC(rtc);

        let rtc_ref = handle.as_webrtc();
        assert!(rtc_ref.is_some());
        assert_eq!(rtc_ref.unwrap().peer_id(), "peer-123");

        // WebSocket 不应该转换成 WebRTC
        let ws = WebSocketConnection::new("ws://test.com");
        let ws_handle = WireHandle::WebSocket(ws);
        assert!(ws_handle.as_webrtc().is_none());
    }

    #[test]
    fn test_wire_handle_as_websocket() {
        let ws = WebSocketConnection::new("ws://example.com");
        let handle = WireHandle::WebSocket(ws);

        let ws_ref = handle.as_websocket();
        assert!(ws_ref.is_some());
        assert_eq!(ws_ref.unwrap().url(), "ws://example.com");

        // WebRTC 不应该转换成 WebSocket
        let rtc = WebRtcConnection::new("peer-456".to_string());
        let rtc_handle = WireHandle::WebRTC(rtc);
        assert!(rtc_handle.as_websocket().is_none());
    }

    #[test]
    fn test_wire_handle_as_webrtc_mut() {
        let rtc = WebRtcConnection::new("peer-mut".to_string());
        let mut handle = WireHandle::WebRTC(rtc);

        let rtc_mut = handle.as_webrtc_mut();
        assert!(rtc_mut.is_some());
    }

    #[wasm_bindgen_test]
    async fn test_wire_handle_is_connected_websocket() {
        let ws = WebSocketConnection::new("ws://test.com");
        let handle = WireHandle::WebSocket(ws);

        assert!(!handle.is_connected());

        handle.connect().await.unwrap();
        assert!(handle.is_connected());
    }

    #[test]
    fn test_wire_handle_is_connected_webrtc() {
        let rtc = WebRtcConnection::new("peer-test".to_string());
        let handle = WireHandle::WebRTC(rtc);

        assert!(!handle.is_connected());
    }

    #[test]
    fn test_wire_handle_clone() {
        let ws = WebSocketConnection::new("ws://clone.com");
        let handle1 = WireHandle::WebSocket(ws);
        let handle2 = handle1.clone();

        match (&handle1, &handle2) {
            (WireHandle::WebSocket(ws1), WireHandle::WebSocket(ws2)) => {
                assert_eq!(ws1.url(), ws2.url());
            }
            _ => panic!("Expected WebSocket handles"),
        }
    }

    #[test]
    fn test_wire_handle_debug() {
        let ws = WebSocketConnection::new("ws://debug.com");
        let handle = WireHandle::WebSocket(ws);

        let debug_str = format!("{:?}", handle);
        assert!(debug_str.contains("WebSocket"));
    }

    // ===== WireStatus 测试 =====

    #[test]
    fn test_wire_status_connecting() {
        let status = WireStatus::Connecting;
        let debug_str = format!("{:?}", status);
        assert!(debug_str.contains("Connecting"));
    }

    #[test]
    fn test_wire_status_ready() {
        let ws = WebSocketConnection::new("ws://test.com");
        let handle = WireHandle::WebSocket(ws);
        let status = WireStatus::Ready(handle);

        match status {
            WireStatus::Ready(h) => {
                assert_eq!(h.conn_type(), ConnType::WebSocket);
            }
            _ => panic!("Expected Ready status"),
        }
    }

    #[test]
    fn test_wire_status_failed() {
        let status = WireStatus::Failed;
        let debug_str = format!("{:?}", status);
        assert!(debug_str.contains("Failed"));
    }

    #[test]
    fn test_wire_status_clone() {
        let status1 = WireStatus::Connecting;
        let status2 = status1.clone();

        match (status1, status2) {
            (WireStatus::Connecting, WireStatus::Connecting) => {}
            _ => panic!("Expected both to be Connecting"),
        }
    }

    #[test]
    fn test_wire_status_all_variants() {
        let ws = WebSocketConnection::new("ws://test.com");
        let handle = WireHandle::WebSocket(ws);

        let _connecting = WireStatus::Connecting;
        let _ready = WireStatus::Ready(handle);
        let _failed = WireStatus::Failed;

        // 验证所有变体都能创建
    }

    // ===== 集成测试 =====

    #[wasm_bindgen_test]
    async fn test_wire_handle_lifecycle_websocket() {
        let ws = WebSocketConnection::new("ws://lifecycle.com");
        let handle = WireHandle::WebSocket(ws);

        // 初始未连接
        assert!(!handle.is_connected());

        // 连接
        handle.connect().await.unwrap();
        assert!(handle.is_connected());

        // 关闭
        handle.close().await.unwrap();
        assert!(!handle.is_connected());
    }

    #[test]
    fn test_wire_handle_type_checks() {
        let ws = WebSocketConnection::new("ws://test.com");
        let ws_handle = WireHandle::WebSocket(ws);

        let rtc = WebRtcConnection::new("peer-test".to_string());
        let rtc_handle = WireHandle::WebRTC(rtc);

        // WebSocket 测试
        assert!(ws_handle.as_websocket().is_some());
        assert!(ws_handle.as_webrtc().is_none());
        assert_eq!(ws_handle.connection_type(), "WebSocket");

        // WebRTC 测试
        assert!(rtc_handle.as_webrtc().is_some());
        assert!(rtc_handle.as_websocket().is_none());
        assert_eq!(rtc_handle.connection_type(), "WebRTC");
    }
}
