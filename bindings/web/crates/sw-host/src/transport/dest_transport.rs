//! Transport manager for a single destination.
//!
//! Manages all connections and message routing for one `Dest` using an
//! event-driven model with no polling.

use super::wire_handle::WireHandle;
use super::wire_pool::WirePool;
use actr_web_common::{ConnType, Dest, PayloadType, WebError, WebResult};
use bytes::Bytes;
use std::sync::Arc;

/// Transport manager for a single destination.
///
/// Responsibilities:
/// - Manage all connections to one `Dest` (`WebSocket` + `WebRTC`)
/// - Start connection attempts concurrently in the background
/// - Wait for connection readiness in an event-driven way
/// - Reuse cached lanes stored inside `WireHandle`
/// - Delegate connection priority selection to `WirePool`
pub struct DestTransport {
    /// Destination.
    dest: Dest,

    /// Connection manager.
    wire_pool: Arc<WirePool>,
}

impl DestTransport {
    /// Create a new `DestTransport`.
    ///
    /// # Parameters
    /// - `dest`: destination
    /// - `connections`: pre-created connection list (`WebSocket`/`WebRTC`), possibly empty
    pub async fn new(dest: Dest, connections: Vec<WireHandle>) -> WebResult<Self> {
        let wire_pool = Arc::new(WirePool::new());

        // Start all connection tasks concurrently in the background.
        log::info!("[{:?}] Starting connection tasks...", dest);
        for conn in connections {
            wire_pool.add_connection(conn);
        }

        Ok(Self { dest, wire_pool })
    }

    /// Get the WirePool reference.
    ///
    /// This allows external code to inject a new `WireHandle` after a
    /// connection becomes available, for example when the DOM passes in a
    /// `MessagePort`.
    pub fn wire_pool(&self) -> &Arc<WirePool> {
        &self.wire_pool
    }

    /// Send a message.
    ///
    /// Event-driven behavior:
    /// - If a connection is available, send immediately
    /// - Otherwise wait for connection state changes through the watcher channel
    /// - `WirePool` already handles priority, so this method only tries lane types in order
    pub async fn send(&self, payload_type: PayloadType, data: &[u8]) -> WebResult<()> {
        log::debug!(
            "[{:?}] Sending message: type={:?}, size={}",
            self.dest,
            payload_type,
            data.len()
        );

        // 1. Determine candidate connection types (simplified: WebRTC first).
        let conn_types = self.get_conn_types_for(payload_type);

        if conn_types.is_empty() {
            return Err(WebError::Transport(format!(
                "No route for: {:?}",
                payload_type
            )));
        }

        // 2. Subscribe to connection state changes.
        let mut watcher = self.wire_pool.subscribe_changes();

        loop {
            // 3. Check the current ready connections snapshot.
            let ready_set = watcher.borrow_and_update();

            log::trace!("[{:?}] Available connections: {:?}", self.dest, ready_set);

            // 4. Try each connection type in priority order.
            for &conn_type in &conn_types {
                // Skip connections that are not ready yet.
                if !ready_set.contains(&conn_type) {
                    log::trace!("[{:?}] {:?} not ready, trying next", self.dest, conn_type);
                    continue;
                }

                // Fetch the connection and build or reuse its lane.
                if let Some(conn) = self.wire_pool.get_connection(conn_type).await {
                    match conn.get_lane(payload_type).await {
                        Ok(lane) => {
                            log::debug!(
                                "[{:?}] Using connection: {:?} (type={:?})",
                                self.dest,
                                conn_type,
                                payload_type
                            );

                            // Convert to `Bytes`.
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

            // 5. All attempts failed, so wait for a connection state change.
            log::info!("[{:?}] Waiting for connection status...", self.dest);

            // Event-driven wait.
            if watcher.changed().await.is_err() {
                return Err(WebError::Transport("connection manager closed".to_string()));
            }

            log::debug!("[{:?}] Connection status updated, retrying...", self.dest);
        }
    }

    /// Close the transport and release all connection resources.
    pub async fn close(&self) -> WebResult<()> {
        log::info!("[{:?}] Closing DestTransport", self.dest);

        // Close all connections.
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

    /// Check whether there is at least one healthy connection.
    ///
    /// Used by health checks to detect complete transport failure.
    ///
    /// # Returns
    /// - `true` if at least one connection is healthy
    /// - `false` if all connections are unhealthy or missing
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

    /// Get connection types for a payload in priority order.
    ///
    /// Simplified rules:
    /// - `MEDIA_RTP`: WebRTC only
    /// - Others: WebRTC first, WebSocket as fallback
    fn get_conn_types_for(&self, payload_type: PayloadType) -> Vec<ConnType> {
        match payload_type {
            PayloadType::MediaRtp => vec![ConnType::WebRTC],
            _ => vec![ConnType::WebRTC, ConnType::WebSocket],
        }
    }

    /// Get the destination.
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

        // Connect first.
        ws.connect().await.unwrap();

        let handle = WireHandle::WebSocket(ws);
        let dt = DestTransport::new(dest, vec![handle]).await.unwrap();

        // Need to wait for the connection-state update.
        gloo_timers::future::TimeoutFuture::new(100).await;

        let healthy = dt.has_healthy_connection().await;
        assert!(healthy);
    }

    #[test]
    fn test_conn_types_priority_order() {
        let dt = create_test_transport();

        // Non-RTP types should prefer WebRTC first.
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
