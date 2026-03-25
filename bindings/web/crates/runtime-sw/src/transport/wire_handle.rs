//! Unified handle for wire-layer components.
//!
//! Provides a single access interface for wire implementations such as
//! WebSocket and WebRTC.

use super::lane::DataLane;
use super::websocket_connection::WebSocketConnection;
use actr_web_common::{ConnType, PayloadType, WebError, WebResult};
use dashmap::DashMap;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;

/// WebRTC connection created by the DOM side.
///
/// The Service Worker stores a `MessagePort` reference while the actual
/// `PeerConnection` lives in the DOM.
#[derive(Clone)]
pub struct WebRtcConnection {
    /// Peer ID.
    peer_id: String,

    /// `MessagePort` for the DataChannel, transferred from the DOM side.
    /// Once the DOM establishes the P2P connection, it sends this port to the SW.
    datachannel_port: Option<Arc<web_sys::MessagePort>>,

    /// Connection state.
    connected: Arc<parking_lot::Mutex<bool>>,

    /// DataLane cache keyed by `PayloadType` to avoid rebuilding lanes.
    lane_cache: Arc<DashMap<PayloadType, DataLane>>,
}

impl WebRtcConnection {
    /// Create a new WebRTC connection handle in the disconnected state.
    pub fn new(peer_id: String) -> Self {
        Self {
            peer_id,
            datachannel_port: None,
            connected: Arc::new(parking_lot::Mutex::new(false)),
            lane_cache: Arc::new(DashMap::new()),
        }
    }

    /// Set the DataChannel `MessagePort` after DOM-side creation completes.
    pub fn set_datachannel_port(&mut self, port: web_sys::MessagePort) {
        self.datachannel_port = Some(Arc::new(port));
        *self.connected.lock() = true;
        log::info!(
            "[WebRtcConnection] DataChannel port set for: {}",
            self.peer_id
        );
    }

    /// Connect the handle.
    pub async fn connect(&self) -> WebResult<()> {
        // The DOM side creates the WebRTC connection. The SW only waits for it.
        // If a port already exists, the connection is considered ready.
        if self.datachannel_port.is_some() {
            Ok(())
        } else {
            Err(WebError::Transport(
                "WebRTC connection not ready (waiting for DOM)".to_string(),
            ))
        }
    }

    /// Check whether the connection is ready.
    pub fn is_connected(&self) -> bool {
        *self.connected.lock()
    }

    /// Get or create a cached `DataLane`.
    ///
    /// WebRTC on the SW side is bridged into the DOM through `MessagePort`:
    ///   SW `DataLane::PostMessage` -> `MessagePort` -> DOM -> `RtcDataChannel`
    ///
    /// The caching strategy matches `WebSocketConnection::get_lane`.
    pub async fn get_lane(&self, payload_type: PayloadType) -> WebResult<DataLane> {
        // 1. Check the cache.
        if let Some(lane) = self.lane_cache.get(&payload_type) {
            log::trace!(
                "[WebRtcConnection] Reusing cached lane: peer={} type={:?}",
                self.peer_id,
                payload_type
            );
            return Ok(lane.clone());
        }

        // 2. A datachannel port is required to create a lane.
        let port = self.datachannel_port.as_ref().ok_or_else(|| {
            WebError::Transport("WebRTC connection not ready (no MessagePort)".to_string())
        })?;

        log::debug!(
            "[WebRtcConnection] Creating PostMessage lane: peer={} type={:?}",
            self.peer_id,
            payload_type
        );

        // 3. Build `DataLane::PostMessage` directly.
        //
        // Send path: SW -> datachannel_port.postMessage() -> DOM bridge -> RtcDataChannel.send()
        // Receive path: DOM bridge -> SW handle_fast_path (without this lane's rx)
        //
        // The rx channel stays empty because receiving is handled via `handle_fast_path`.
        let (_tx, rx) = mpsc::unbounded();
        let lane = DataLane::PostMessage {
            port: Arc::clone(port),
            payload_type,
            rx: Arc::new(Mutex::new(rx)),
            failure_notifier: None,
        };

        // 4. Cache the lane.
        self.lane_cache.insert(payload_type, lane.clone());

        log::info!(
            "[WebRtcConnection] DataLane created: peer={} type={:?}",
            self.peer_id,
            payload_type
        );

        Ok(lane)
    }

    /// Close the connection.
    pub async fn close(&self) -> WebResult<()> {
        *self.connected.lock() = false;
        self.lane_cache.clear();
        log::info!("[WebRtcConnection] Closed: {}", self.peer_id);
        Ok(())
    }

    /// Get the peer ID.
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

/// Unified handle for wire-layer components.
///
/// # Design
/// - Uses enum dispatch instead of trait objects to avoid virtual dispatch cost
/// - Provides a uniform interface over different wire implementations
/// - Supports connection priority ordering (`WebRTC` > `WebSocket`)
#[derive(Clone, Debug)]
pub enum WireHandle {
    /// WebSocket connection handle.
    WebSocket(WebSocketConnection),

    /// WebRTC connection handle.
    WebRTC(WebRtcConnection),
}

impl WireHandle {
    /// Get the connection type.
    #[inline]
    pub fn conn_type(&self) -> ConnType {
        match self {
            WireHandle::WebSocket(_) => ConnType::WebSocket,
            WireHandle::WebRTC(_) => ConnType::WebRTC,
        }
    }

    /// Get the connection type name.
    #[inline]
    pub fn connection_type(&self) -> &'static str {
        match self {
            WireHandle::WebSocket(_) => "WebSocket",
            WireHandle::WebRTC(_) => "WebRTC",
        }
    }

    /// Connection priority, where higher values mean higher priority.
    #[inline]
    pub fn priority(&self) -> u8 {
        match self {
            WireHandle::WebSocket(_) => 0,
            WireHandle::WebRTC(_) => 1, // WebRTC has higher priority.
        }
    }

    /// Connect.
    #[inline]
    pub async fn connect(&self) -> WebResult<()> {
        match self {
            WireHandle::WebSocket(ws) => ws.connect().await,
            WireHandle::WebRTC(rtc) => rtc.connect().await,
        }
    }

    /// Check whether the connection is ready.
    #[inline]
    pub fn is_connected(&self) -> bool {
        match self {
            WireHandle::WebSocket(ws) => ws.is_connected(),
            WireHandle::WebRTC(rtc) => rtc.is_connected(),
        }
    }

    /// Close the connection.
    #[inline]
    pub async fn close(&self) -> WebResult<()> {
        match self {
            WireHandle::WebSocket(ws) => ws.close().await,
            WireHandle::WebRTC(rtc) => rtc.close().await,
        }
    }

    /// Get or create a cached `DataLane`.
    #[inline]
    pub async fn get_lane(&self, payload_type: PayloadType) -> WebResult<DataLane> {
        match self {
            WireHandle::WebSocket(ws) => ws.get_lane(payload_type).await,
            WireHandle::WebRTC(rtc) => rtc.get_lane(payload_type).await,
        }
    }

    /// Cast to a WebRTC connection if possible.
    #[inline]
    pub fn as_webrtc(&self) -> Option<&WebRtcConnection> {
        match self {
            WireHandle::WebRTC(rtc) => Some(rtc),
            _ => None,
        }
    }

    /// Cast to a mutable WebRTC connection if possible.
    #[inline]
    pub fn as_webrtc_mut(&mut self) -> Option<&mut WebRtcConnection> {
        match self {
            WireHandle::WebRTC(rtc) => Some(rtc),
            _ => None,
        }
    }

    /// Cast to a WebSocket connection if possible.
    #[inline]
    pub fn as_websocket(&self) -> Option<&WebSocketConnection> {
        match self {
            WireHandle::WebSocket(ws) => Some(ws),
            _ => None,
        }
    }
}

/// Wire connection state.
#[derive(Debug, Clone)]
pub enum WireStatus {
    /// Connecting.
    Connecting,

    /// Ready.
    Ready(WireHandle),

    /// Failed.
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // ===== WebRtcConnection tests =====

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

    // ===== WireHandle tests =====

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

        // WebRTC has higher priority.
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

        // WebSocket should not cast to WebRTC.
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

        // WebRTC should not cast to WebSocket.
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

    // ===== WireStatus tests =====

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

        // Verify that every variant can be constructed.
    }

    // ===== Integration tests =====

    #[wasm_bindgen_test]
    async fn test_wire_handle_lifecycle_websocket() {
        let ws = WebSocketConnection::new("ws://lifecycle.com");
        let handle = WireHandle::WebSocket(ws);

        // Initially disconnected.
        assert!(!handle.is_connected());

        // Connect.
        handle.connect().await.unwrap();
        assert!(handle.is_connected());

        // Close.
        handle.close().await.unwrap();
        assert!(!handle.is_connected());
    }

    #[test]
    fn test_wire_handle_type_checks() {
        let ws = WebSocketConnection::new("ws://test.com");
        let ws_handle = WireHandle::WebSocket(ws);

        let rtc = WebRtcConnection::new("peer-test".to_string());
        let rtc_handle = WireHandle::WebRTC(rtc);

        // WebSocket test.
        assert!(ws_handle.as_websocket().is_some());
        assert!(ws_handle.as_webrtc().is_none());
        assert_eq!(ws_handle.connection_type(), "WebSocket");

        // WebRTC test.
        assert!(rtc_handle.as_webrtc().is_some());
        assert!(rtc_handle.as_websocket().is_none());
        assert_eq!(rtc_handle.connection_type(), "WebRTC");
    }
}
