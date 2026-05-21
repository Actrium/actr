//! Wire builder - factory for Wire-layer connections.
//!
//! Uses a factory pattern to create Wire-layer connections such as WebSocket and WebRTC.

use super::websocket_connection::WebSocketConnection;
use super::wire_handle::WireHandle;
use actr_web_common::{Dest, WebResult};
use async_trait::async_trait;

/// Wire builder trait for asynchronously creating Wire components from `Dest`.
///
/// Implement this trait to customize Wire-layer connection creation such as WebRTC or WebSocket.
#[async_trait(?Send)] // The WASM runtime does not support `Send`.
pub trait WireBuilder {
    /// Create connections for the given destination.
    ///
    /// # Parameters
    /// - `dest`: Target destination
    ///
    /// # Returns
    /// - List of Wire handles, potentially including multiple transport types
    async fn create_connections(&self, dest: &Dest) -> WebResult<Vec<WireHandle>>;
}

/// WireBuilder implementation for the Web environment.
///
/// Stateless: WebSocket destinations are turned into a `WebSocketConnection`,
/// while peer destinations are owned by the DOM-side JS WebRTC coordinator
/// (see `actor.sw.js` + `actr-dom`); the resulting `WireHandle` is injected
/// separately once the DataChannel is ready.
pub struct WebWireBuilder;

impl WebWireBuilder {
    /// Create a new WebWireBuilder.
    pub fn new() -> Self {
        Self
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
                // Create a WebSocket directly.
                log::debug!("[WebWireBuilder] Creating WebSocket to: {}", url);
                let ws = WebSocketConnection::new(url);
                connections.push(WireHandle::WebSocket(ws));
            }

            Dest::Peer(peer_id) => {
                // Peer connections are created only through WebRTC DataChannel on the DOM side.
                // No WebSocket fallback is created here; relay behavior is handled via signaling.
                //
                // The DOM-side WebRTC coordinator runs in JS (see actor.sw.js + actr-dom);
                // Rust does not currently dispatch a P2P creation request — the JS layer
                // observes the peer destination and arranges the DataChannel out of band.
                log::debug!(
                    "[WebWireBuilder] Peer dest {} — DOM-side JS coordinator handles P2P setup; \
                     a WireHandle is injected separately once it is ready",
                    peer_id
                );
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
        let _builder = WebWireBuilder::new();
    }

    #[test]
    fn test_web_wire_builder_default() {
        let _builder = <WebWireBuilder as Default>::default();
    }

    #[wasm_bindgen_test]
    async fn test_create_connections_server() {
        let builder = WebWireBuilder::new();
        let dest = Dest::Server("wss://server.example.com".to_string());

        let result = builder.create_connections(&dest).await;
        assert!(result.is_ok());

        let connections = result.unwrap();
        assert_eq!(connections.len(), 1);

        // Verify it is a WebSocket.
        match &connections[0] {
            WireHandle::WebSocket(ws) => {
                assert_eq!(ws.url(), "wss://server.example.com");
            }
            _ => panic!("Expected WebSocket connection"),
        }
    }

    #[wasm_bindgen_test]
    async fn test_create_connections_peer() {
        // Dest::Peer never returns a WireHandle synchronously — the DOM side
        // injects the WebRTC DataChannel asynchronously.
        let builder = WebWireBuilder::new();
        let dest = Dest::Peer("peer-123".to_string());

        let result = builder.create_connections(&dest).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[wasm_bindgen_test]
    async fn test_create_connections_multiple_dests() {
        let builder = WebWireBuilder::new();

        let dest1 = Dest::Server("wss://server1.com".to_string());
        let result1 = builder.create_connections(&dest1).await;
        assert!(result1.is_ok());
        assert_eq!(result1.unwrap().len(), 1);

        // Dest::Peer returns no synchronous connections (see test_create_connections_peer).
        let dest2 = Dest::Peer("peer-456".to_string());
        let result2 = builder.create_connections(&dest2).await;
        assert!(result2.is_ok());
        assert!(result2.unwrap().is_empty());
    }
}
