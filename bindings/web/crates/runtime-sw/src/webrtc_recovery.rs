//! WebRTC connection recovery management.
//!
//! Rebuilds WebRTC connectivity after the DOM side restarts.

use crate::{PeerTransport, WebResult, WirePool};
use actr_web_common::ConnType;
use std::sync::Arc;
use web_sys::MessagePort;

/// WebRTC recovery manager.
pub struct WebRtcRecoveryManager {
    /// WirePool reference.
    wire_pool: Arc<WirePool>,

    /// Optional transport manager reference.
    transport_manager: Option<Arc<PeerTransport>>,
}

impl WebRtcRecoveryManager {
    /// Create a new recovery manager.
    pub fn new(wire_pool: Arc<WirePool>) -> Self {
        log::info!("[WebRtcRecovery] Creating recovery manager");

        Self {
            wire_pool,
            transport_manager: None,
        }
    }

    /// Attach a transport manager.
    pub fn with_transport_manager(mut self, manager: Arc<PeerTransport>) -> Self {
        self.transport_manager = Some(manager);
        self
    }

    /// Handle a DOM restart event.
    ///
    /// Called when the DOM side sends a `"DOM_READY"` message.
    pub async fn handle_dom_restart(&self, session_id: String) -> WebResult<()> {
        log::info!(
            "[WebRtcRecovery] Handling DOM restart: session_id={}",
            session_id
        );

        // 1. Clear all WebRTC connections.
        self.cleanup_stale_connections();

        // 2. Ask the DOM side to re-establish WebRTC.
        // The real rebuild request must go through a control channel.
        // This currently only logs and depends on the DOM communication layer.
        log::info!("[WebRtcRecovery] WebRTC connections cleaned, waiting for DOM to rebuild");

        Ok(())
    }

    /// Remove stale connections.
    fn cleanup_stale_connections(&self) {
        log::info!("[WebRtcRecovery] Cleaning up stale WebRTC connections");

        // Remove WebRTC connections.
        self.wire_pool.remove_connection(ConnType::WebRTC);

        log::info!("[WebRtcRecovery] Stale connections removed");
    }

    /// Register a new `MessagePort`.
    ///
    /// After the DOM rebuilds WebRTC, it sends a fresh `MessagePort` to the SW.
    pub fn register_new_port(&self, peer_id: String, port: MessagePort) -> WebResult<()> {
        log::info!(
            "[WebRtcRecovery] Registering new MessagePort for peer: {}",
            peer_id
        );

        // Build a new WebRTC connection.
        use crate::transport::WebRtcConnection;

        let mut rtc_conn = WebRtcConnection::new(peer_id.clone());
        rtc_conn.set_datachannel_port(port);

        // Add it back to the WirePool.
        use crate::transport::WireHandle;
        self.wire_pool.reconnect(WireHandle::WebRTC(rtc_conn));

        log::info!(
            "[WebRtcRecovery] WebRTC connection rebuilt successfully for peer: {}",
            peer_id
        );

        Ok(())
    }

    /// Request that the DOM rebuild WebRTC.
    ///
    /// Sends a rebuild request message to the DOM side.
    #[allow(dead_code)]
    async fn request_webrtc_rebuild(&self) -> WebResult<()> {
        log::info!("[WebRtcRecovery] Requesting WebRTC rebuild from DOM");

        // TODO: Implement control-message delivery.
        // This needs a dedicated control channel separate from `DataLane`.
        // Possible options:
        // 1. A dedicated `MessagePort` used as a control port
        // 2. Broadcast via `ServiceWorker.postMessage`
        // 3. Send a special control message through the existing DOM lane

        log::warn!("[WebRtcRecovery] Control channel not implemented yet");

        Ok(())
    }

    /// Check recovery status.
    pub async fn check_recovery_status(&self) -> RecoveryStatus {
        let health = self.wire_pool.health_check().await;

        let webrtc_healthy = health.get(&ConnType::WebRTC).copied().unwrap_or(false);
        let websocket_healthy = health.get(&ConnType::WebSocket).copied().unwrap_or(false);

        RecoveryStatus {
            webrtc_connected: webrtc_healthy,
            websocket_connected: websocket_healthy,
            needs_recovery: !webrtc_healthy,
        }
    }

    /// Get the WirePool reference.
    pub fn wire_pool(&self) -> &Arc<WirePool> {
        &self.wire_pool
    }
}

/// Recovery status.
#[derive(Debug, Clone)]
pub struct RecoveryStatus {
    /// Whether WebRTC is connected.
    pub webrtc_connected: bool,

    /// Whether WebSocket is connected.
    pub websocket_connected: bool,

    /// Whether recovery is needed.
    pub needs_recovery: bool,
}

impl RecoveryStatus {
    /// Whether the transport is healthy.
    pub fn is_healthy(&self) -> bool {
        self.webrtc_connected || self.websocket_connected
    }

    /// Whether recovery is complete.
    pub fn is_fully_recovered(&self) -> bool {
        self.webrtc_connected && self.websocket_connected
    }
}

#[cfg(test)]
#[allow(clippy::arc_with_non_send_sync)]
mod tests {
    use super::*;

    fn create_test_wire_pool() -> Arc<WirePool> {
        Arc::new(WirePool::new())
    }

    #[test]
    fn test_recovery_status() {
        let status = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: true,
            needs_recovery: false,
        };

        assert!(status.is_healthy());
        assert!(status.is_fully_recovered());

        let status2 = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };

        assert!(status2.is_healthy());
        assert!(!status2.is_fully_recovered());

        let status3 = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: false,
            needs_recovery: true,
        };

        assert!(!status3.is_healthy());
        assert!(!status3.is_fully_recovered());
    }

    #[test]
    fn test_recovery_manager_creation() {
        let wire_pool = create_test_wire_pool();
        let _manager = WebRtcRecoveryManager::new(wire_pool);
        // Creation alone is sufficient for this test.
    }

    #[test]
    fn test_recovery_manager_with_transport_manager() {
        let wire_pool = create_test_wire_pool();
        let wire_builder = Arc::new(crate::transport::WebWireBuilder::new());
        let transport_manager = Arc::new(PeerTransport::new("test-sw".to_string(), wire_builder));

        let manager =
            WebRtcRecoveryManager::new(wire_pool).with_transport_manager(transport_manager);

        // Verify the transport manager is attached correctly.
        assert!(manager.transport_manager.is_some());
    }

    #[test]
    fn test_recovery_status_is_healthy_with_webrtc_only() {
        let status = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: false,
            needs_recovery: false,
        };

        assert!(status.is_healthy());
        assert!(!status.is_fully_recovered());
    }

    #[test]
    fn test_recovery_status_is_healthy_with_websocket_only() {
        let status = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };

        assert!(status.is_healthy());
        assert!(!status.is_fully_recovered());
    }

    #[test]
    fn test_recovery_status_needs_recovery() {
        // Recovery is needed when WebRTC is disconnected.
        let status = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };

        assert!(status.needs_recovery);

        // No recovery is needed when both are connected.
        let status2 = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: true,
            needs_recovery: false,
        };

        assert!(!status2.needs_recovery);
    }

    #[test]
    fn test_recovery_status_all_disconnected() {
        let status = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: false,
            needs_recovery: true,
        };

        assert!(!status.is_healthy());
        assert!(!status.is_fully_recovered());
        assert!(status.needs_recovery);
    }

    #[test]
    fn test_recovery_status_clone() {
        let status1 = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: false,
            needs_recovery: false,
        };

        let status2 = status1.clone();

        assert_eq!(status1.webrtc_connected, status2.webrtc_connected);
        assert_eq!(status1.websocket_connected, status2.websocket_connected);
        assert_eq!(status1.needs_recovery, status2.needs_recovery);
    }

    #[test]
    fn test_wire_pool_reference() {
        let wire_pool = create_test_wire_pool();
        let manager = WebRtcRecoveryManager::new(wire_pool.clone());

        // Verify that the WirePool reference can be retrieved.
        let pool_ref = manager.wire_pool();
        assert!(Arc::ptr_eq(pool_ref, &wire_pool));
    }

    #[test]
    fn test_recovery_status_debug() {
        let status = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: false,
            needs_recovery: true,
        };

        let debug_str = format!("{:?}", status);
        assert!(debug_str.contains("RecoveryStatus"));
        assert!(debug_str.contains("webrtc_connected"));
        assert!(debug_str.contains("websocket_connected"));
    }

    #[test]
    fn test_recovery_status_all_combinations() {
        // Test all key combinations.
        let combinations = vec![
            (false, false, true), // Both disconnected
            (false, true, true),  // WebSocket only
            (true, false, false), // WebRTC only
            (true, true, false),  // Both connected
        ];

        for (webrtc, websocket, recovery) in combinations {
            let status = RecoveryStatus {
                webrtc_connected: webrtc,
                websocket_connected: websocket,
                needs_recovery: recovery,
            };

            // Verify logical consistency.
            if webrtc || websocket {
                assert!(status.is_healthy());
            } else {
                assert!(!status.is_healthy());
            }

            if webrtc && websocket {
                assert!(status.is_fully_recovered());
            } else {
                assert!(!status.is_fully_recovered());
            }
        }
    }

    #[test]
    fn test_recovery_manager_default_state() {
        let wire_pool = create_test_wire_pool();
        let manager = WebRtcRecoveryManager::new(wire_pool);

        // Verify the initial state.
        assert!(manager.transport_manager.is_none());
    }

    #[test]
    fn test_with_transport_manager_builder_pattern() {
        let wire_pool = create_test_wire_pool();
        let wire_builder = Arc::new(crate::transport::WebWireBuilder::new());
        let transport_manager = Arc::new(PeerTransport::new("test-sw".to_string(), wire_builder));

        // Use the builder pattern.
        let manager =
            WebRtcRecoveryManager::new(wire_pool.clone()).with_transport_manager(transport_manager);

        assert!(manager.transport_manager.is_some());

        // Verify that `wire_pool` can be retrieved.
        let pool_ref = manager.wire_pool();
        assert!(Arc::ptr_eq(pool_ref, &wire_pool));
    }

    #[test]
    fn test_recovery_status_needs_recovery_logic() {
        // Recovery is needed when WebRTC is disconnected.
        let status1 = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };
        assert!(status1.needs_recovery);

        // No recovery is needed when both are connected.
        let status2 = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: true,
            needs_recovery: false,
        };
        assert!(!status2.needs_recovery);
    }

    #[test]
    fn test_recovery_status_equality() {
        let status1 = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: false,
            needs_recovery: false,
        };

        let status2 = status1.clone();

        // Verify equality after cloning.
        assert_eq!(status1.webrtc_connected, status2.webrtc_connected);
        assert_eq!(status1.websocket_connected, status2.websocket_connected);
        assert_eq!(status1.needs_recovery, status2.needs_recovery);
    }

    #[test]
    fn test_cleanup_stale_connections() {
        let wire_pool = create_test_wire_pool();
        let manager = WebRtcRecoveryManager::new(wire_pool.clone());

        // Call `cleanup_stale_connections` through the internal method.
        // This test only verifies that the method does not panic.
        manager.cleanup_stale_connections();
    }

    #[test]
    fn test_recovery_manager_multiple_instances() {
        let wire_pool1 = create_test_wire_pool();
        let wire_pool2 = create_test_wire_pool();

        let manager1 = WebRtcRecoveryManager::new(wire_pool1.clone());
        let manager2 = WebRtcRecoveryManager::new(wire_pool2.clone());

        // Verify that each manager owns an independent WirePool.
        assert!(!Arc::ptr_eq(manager1.wire_pool(), manager2.wire_pool()));
    }

    #[test]
    fn test_recovery_status_partial_recovery_scenarios() {
        // Scenario 1: WebRTC is recovering while WebSocket is connected.
        let status = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };
        assert!(status.is_healthy()); // Any live connection counts as healthy.
        assert!(!status.is_fully_recovered()); // But recovery is not complete.
        assert!(status.needs_recovery);

        // Scenario 2: WebSocket is recovering while WebRTC is connected.
        let status2 = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: false,
            needs_recovery: false, // Recovery may not be needed because WebRTC is primary.
        };
        assert!(status2.is_healthy());
        assert!(!status2.is_fully_recovered());
    }

    #[test]
    fn test_recovery_manager_wire_pool_access() {
        let wire_pool = create_test_wire_pool();
        let manager = WebRtcRecoveryManager::new(wire_pool.clone());

        // Verify that `wire_pool` is accessible.
        let pool_ref = manager.wire_pool();
        assert!(Arc::ptr_eq(pool_ref, &wire_pool));

        // Repeated access should return the same reference.
        let pool_ref2 = manager.wire_pool();
        assert!(Arc::ptr_eq(pool_ref, pool_ref2));
    }
}
