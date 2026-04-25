//! WebRTC coordinator for the DOM side.
//!
//! Historical role: receive P2P creation requests from the Service Worker, drive
//! `RTCPeerConnection` setup, and notify the SW when ready. The SW↔DOM control
//! plane has since moved into JS (see `actor.sw.js` + `actr-dom`), so this Rust
//! coordinator no longer carries the event loop or the request handlers — see
//! TD-001 in `bindings/web/docs/tech-debt.zh.md`.
//!
//! What remains is a thin, no-op holder for the peer-connection registry plus
//! the close helpers, kept so the `pub` API surface stays stable for downstream
//! crates.

use actr_web_common::WebResult;
use dashmap::DashMap;
use std::sync::Arc;
use web_sys::RtcPeerConnection;

/// WebRTC coordinator for the DOM side.
///
/// This is a pure helper role: it does not manage live connections and only
/// holds a peer-connection registry that downstream code can close.
#[allow(dead_code)]
pub struct WebRtcCoordinator {
    /// Active PeerConnections keyed by peer ID.
    peer_connections: Arc<DashMap<String, RtcPeerConnection>>,

    /// ICE server configuration.
    ice_servers: Vec<String>,
}

impl WebRtcCoordinator {
    /// Create a new WebRTC coordinator.
    pub fn new(ice_servers: Vec<String>) -> Self {
        Self {
            peer_connections: Arc::new(DashMap::new()),
            ice_servers,
        }
    }

    /// Close the connection for the given peer.
    pub fn close_peer(&self, peer_id: &str) -> WebResult<()> {
        if let Some((_, pc)) = self.peer_connections.remove(peer_id) {
            pc.close();
            log::info!("[WebRtcCoordinator] Closed peer connection: {}", peer_id);
        }
        Ok(())
    }

    /// Close all connections.
    pub fn close_all(&self) -> WebResult<()> {
        log::info!(
            "[WebRtcCoordinator] Closing all peer connections (count: {})",
            self.peer_connections.len()
        );

        for entry in self.peer_connections.iter() {
            entry.value().close();
        }

        self.peer_connections.clear();
        Ok(())
    }
}

impl Default for WebRtcCoordinator {
    fn default() -> Self {
        Self::new(vec!["stun:stun.l.google.com:19302".to_string()])
    }
}
