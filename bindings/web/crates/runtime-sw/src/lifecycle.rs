//! Service Worker lifecycle management.
//!
//! Listens for DOM lifecycle events and coordinates cleanup and recovery.

use crate::error_handler::get_global_error_handler;
use crate::{ConnType, WebError, WebResult, WirePool};
use actr_web_common::ControlMessage;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{ExtendableMessageEvent, ServiceWorkerGlobalScope};

/// Service Worker lifecycle manager.
pub struct SwLifecycleManager {
    /// Set of currently active DOM sessions.
    active_sessions: Arc<Mutex<HashSet<String>>>,

    /// Optional WirePool reference used to clean up stale connections.
    wire_pool: Option<Arc<WirePool>>,
}

impl SwLifecycleManager {
    /// Create a new lifecycle manager.
    pub fn new() -> Self {
        log::info!("[SwLifecycle] Creating lifecycle manager");

        Self {
            active_sessions: Arc::new(Mutex::new(HashSet::new())),
            wire_pool: None,
        }
    }

    /// Set the WirePool reference.
    ///
    /// Used to clean up stale WebRTC connections when the DOM restarts.
    pub fn set_wire_pool(&mut self, wire_pool: Arc<WirePool>) {
        log::info!("[SwLifecycle] WirePool registered");
        self.wire_pool = Some(wire_pool);
    }

    /// Initialize lifecycle management.
    ///
    /// Installs the global message listener.
    pub fn init(&self) -> WebResult<()> {
        log::info!("[SwLifecycle] Initializing lifecycle management");

        self.setup_message_listener()?;

        log::info!("[SwLifecycle] Lifecycle management initialized");
        Ok(())
    }

    /// Set up the global Service Worker message listener.
    fn setup_message_listener(&self) -> WebResult<()> {
        let active_sessions = Arc::clone(&self.active_sessions);
        let wire_pool = self.wire_pool.clone();

        // Get ServiceWorkerGlobalScope.
        let global = js_sys::global();
        let sw_global = global
            .dyn_into::<ServiceWorkerGlobalScope>()
            .map_err(|_| WebError::Internal("Not in Service Worker context".into()))?;

        // Create the message handler callback.
        let callback = Closure::wrap(Box::new(move |event: ExtendableMessageEvent| {
            let data = event.data();

            // First try to handle the payload as a serialized ControlMessage from the DOM error reporter.
            if data.dyn_ref::<js_sys::Object>().is_some() {
                // Try to deserialize it as `Vec<u8>` using serde_wasm_bindgen format.
                if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(data.clone()) {
                    if let Ok(control_msg) = ControlMessage::deserialize(&bytes) {
                        match control_msg {
                            ControlMessage::ErrorReport(error_report) => {
                                log::debug!(
                                    "[SwLifecycle] Received error report via SW controller: {:?}",
                                    error_report.category
                                );

                                // Forward it to the global error handler.
                                if let Some(handler) = get_global_error_handler() {
                                    handler.handle_error_report(error_report);
                                } else {
                                    log::warn!(
                                        "[SwLifecycle] Error handler not initialized, cannot process error report"
                                    );
                                }
                                return;
                            }
                            _ => {
                                // Other ControlMessage types fall through to the normal path.
                            }
                        }
                    }
                }
            }

            // Otherwise try to process it as a plain lifecycle object.
            if let Ok(data_obj) = data.dyn_into::<js_sys::Object>() {
                // Extract the message type.
                if let Ok(msg_type_js) = js_sys::Reflect::get(&data_obj, &"type".into()) {
                    if let Some(msg_type) = msg_type_js.as_string() {
                        // Extract session_id.
                        let session_id = if let Ok(session_id_js) =
                            js_sys::Reflect::get(&data_obj, &"session_id".into())
                        {
                            session_id_js.as_string().unwrap_or_default()
                        } else {
                            String::new()
                        };

                        // Handle each lifecycle message type.
                        match msg_type.as_str() {
                            "DOM_READY" => {
                                Self::handle_dom_ready(&active_sessions, &wire_pool, &session_id);
                            }
                            "DOM_UNLOADING" => {
                                Self::handle_dom_unloading(&active_sessions, &session_id);
                            }
                            "DOM_PING" => {
                                Self::handle_dom_ping(&session_id);
                            }
                            _ => {
                                // Ignore other messages.
                            }
                        }
                    }
                }
            }
        }) as Box<dyn FnMut(ExtendableMessageEvent)>);

        // Register the callback on the SW `message` event.
        sw_global
            .add_event_listener_with_callback("message", callback.as_ref().unchecked_ref())
            .map_err(|e| WebError::Internal(format!("Failed to add message listener: {:?}", e)))?;

        // Keep the callback alive.
        callback.forget();

        log::info!("[SwLifecycle] Message listener registered");
        Ok(())
    }

    /// Handle the `DOM_READY` message.
    ///
    /// Sent after the DOM process restarts.
    fn handle_dom_ready(
        active_sessions: &Arc<Mutex<HashSet<String>>>,
        wire_pool: &Option<Arc<WirePool>>,
        session_id: &str,
    ) {
        if session_id.is_empty() {
            log::warn!("[SwLifecycle] DOM_READY received without session_id");
            return;
        }

        log::info!("[SwLifecycle] DOM_READY received: {}", session_id);

        // Add it to the active session set.
        {
            let mut sessions = active_sessions.lock();
            sessions.insert(session_id.to_string());
        }

        // Clean up stale WebRTC connections.
        if let Some(pool) = wire_pool {
            Self::cleanup_stale_webrtc_connections(pool, session_id);
        } else {
            log::warn!("[SwLifecycle] No WirePool available for cleanup");
        }
    }

    /// Handle the `DOM_UNLOADING` message.
    ///
    /// Sent when the DOM process is about to shut down.
    fn handle_dom_unloading(active_sessions: &Arc<Mutex<HashSet<String>>>, session_id: &str) {
        if session_id.is_empty() {
            log::warn!("[SwLifecycle] DOM_UNLOADING received without session_id");
            return;
        }

        log::info!("[SwLifecycle] DOM_UNLOADING received: {}", session_id);

        // Remove it from the active session set.
        {
            let mut sessions = active_sessions.lock();
            sessions.remove(session_id);
        }

        log::info!("[SwLifecycle] Session {} marked for cleanup", session_id);
    }

    /// Handle the `DOM_PING` message.
    ///
    /// Used by the DOM side to check whether the Service Worker is alive.
    fn handle_dom_ping(session_id: &str) {
        log::debug!("[SwLifecycle] DOM_PING received from {}", session_id);

        // TODO: send a PONG response once there is a return channel.
    }

    /// Clean up stale WebRTC connections.
    fn cleanup_stale_webrtc_connections(wire_pool: &Arc<WirePool>, session_id: &str) {
        log::info!(
            "[SwLifecycle] Cleaning up stale WebRTC connections for session: {}",
            session_id
        );

        // Mark WebRTC connections as failed.
        // This currently removes all WebRTC connections, though finer-grained session
        // management may be needed later.
        wire_pool.mark_connection_failed(ConnType::WebRTC);

        log::info!("[SwLifecycle] WebRTC connections marked as failed");
    }

    /// Return the number of active sessions.
    pub fn active_session_count(&self) -> usize {
        self.active_sessions.lock().len()
    }

    /// Check whether a session is active.
    pub fn is_session_active(&self, session_id: &str) -> bool {
        self.active_sessions.lock().contains(session_id)
    }
}

impl Default for SwLifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_wire_pool() -> Arc<WirePool> {
        Arc::new(WirePool::new())
    }

    #[test]
    fn test_lifecycle_manager_creation() {
        let manager = SwLifecycleManager::new();
        assert_eq!(manager.active_session_count(), 0);
        assert!(manager.wire_pool.is_none());
    }

    #[test]
    fn test_session_tracking() {
        let manager = SwLifecycleManager::new();

        // Simulate adding sessions.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("session-1".to_string());
            sessions.insert("session-2".to_string());
        }

        assert_eq!(manager.active_session_count(), 2);
        assert!(manager.is_session_active("session-1"));
        assert!(manager.is_session_active("session-2"));
        assert!(!manager.is_session_active("session-3"));

        // Remove a session.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.remove("session-1");
        }

        assert_eq!(manager.active_session_count(), 1);
        assert!(!manager.is_session_active("session-1"));
        assert!(manager.is_session_active("session-2"));
    }

    #[test]
    fn test_default_implementation() {
        let manager = SwLifecycleManager::default();
        assert_eq!(manager.active_session_count(), 0);
        assert!(manager.wire_pool.is_none());
    }

    #[test]
    fn test_set_wire_pool() {
        let mut manager = SwLifecycleManager::new();
        let wire_pool = create_test_wire_pool();

        manager.set_wire_pool(wire_pool.clone());

        assert!(manager.wire_pool.is_some());
        assert!(Arc::ptr_eq(&manager.wire_pool.unwrap(), &wire_pool));
    }

    #[test]
    fn test_active_session_count_empty() {
        let manager = SwLifecycleManager::new();
        assert_eq!(manager.active_session_count(), 0);
    }

    #[test]
    fn test_active_session_count_multiple() {
        let manager = SwLifecycleManager::new();

        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("s1".to_string());
            sessions.insert("s2".to_string());
            sessions.insert("s3".to_string());
        }

        assert_eq!(manager.active_session_count(), 3);
    }

    #[test]
    fn test_is_session_active_nonexistent() {
        let manager = SwLifecycleManager::new();
        assert!(!manager.is_session_active("nonexistent"));
    }

    #[test]
    fn test_is_session_active_after_add() {
        let manager = SwLifecycleManager::new();

        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("test-session".to_string());
        }

        assert!(manager.is_session_active("test-session"));
    }

    #[test]
    fn test_is_session_active_after_remove() {
        let manager = SwLifecycleManager::new();

        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("test-session".to_string());
        }

        assert!(manager.is_session_active("test-session"));

        {
            let mut sessions = manager.active_sessions.lock();
            sessions.remove("test-session");
        }

        assert!(!manager.is_session_active("test-session"));
    }

    #[test]
    fn test_handle_dom_ready_with_wire_pool() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));
        let wire_pool = create_test_wire_pool();
        let wire_pool_opt = Some(wire_pool.clone());

        SwLifecycleManager::handle_dom_ready(&active_sessions, &wire_pool_opt, "session-123");

        // Verify that the session was added.
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-123"));
    }

    #[test]
    fn test_handle_dom_ready_without_wire_pool() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));
        let wire_pool_opt: Option<Arc<WirePool>> = None;

        SwLifecycleManager::handle_dom_ready(&active_sessions, &wire_pool_opt, "session-456");

        // The session should still be added even without a wire pool.
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-456"));
    }

    #[test]
    fn test_handle_dom_ready_empty_session_id() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));
        let wire_pool = create_test_wire_pool();
        let wire_pool_opt = Some(wire_pool);

        SwLifecycleManager::handle_dom_ready(&active_sessions, &wire_pool_opt, "");

        // An empty `session_id` should not be added.
        let sessions = active_sessions.lock();
        assert!(!sessions.contains(""));
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn test_handle_dom_unloading() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));

        // Add a session first.
        {
            let mut sessions = active_sessions.lock();
            sessions.insert("session-abc".to_string());
        }

        // Handle `DOM_UNLOADING`.
        SwLifecycleManager::handle_dom_unloading(&active_sessions, "session-abc");

        // Verify that the session was removed.
        let sessions = active_sessions.lock();
        assert!(!sessions.contains("session-abc"));
    }

    #[test]
    fn test_handle_dom_unloading_empty_session_id() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));

        // Add a session.
        {
            let mut sessions = active_sessions.lock();
            sessions.insert("session-xyz".to_string());
        }

        // Handle an empty `session_id`.
        SwLifecycleManager::handle_dom_unloading(&active_sessions, "");

        // Existing sessions should remain unaffected.
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-xyz"));
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn test_handle_dom_unloading_nonexistent_session() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));

        // Add a session.
        {
            let mut sessions = active_sessions.lock();
            sessions.insert("session-1".to_string());
        }

        // Try to remove a non-existent session.
        SwLifecycleManager::handle_dom_unloading(&active_sessions, "session-999");

        // Existing sessions should remain unaffected.
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-1"));
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn test_handle_dom_ping() {
        // Handling `DOM_PING` should not crash.
        SwLifecycleManager::handle_dom_ping("ping-session");

        // TODO: Add stronger assertions once PONG responses are implemented.
    }

    #[test]
    fn test_cleanup_stale_webrtc_connections() {
        let wire_pool = create_test_wire_pool();

        // Clean up stale connections; this should not crash.
        SwLifecycleManager::cleanup_stale_webrtc_connections(&wire_pool, "test-session");

        // Passing the test means the cleanup function executed normally.
    }

    #[test]
    fn test_multiple_sessions_management() {
        let manager = SwLifecycleManager::new();

        // Add multiple sessions.
        {
            let mut sessions = manager.active_sessions.lock();
            for i in 0..10 {
                sessions.insert(format!("session-{}", i));
            }
        }

        assert_eq!(manager.active_session_count(), 10);

        // Verify that all sessions are active.
        for i in 0..10 {
            assert!(manager.is_session_active(&format!("session-{}", i)));
        }

        // Remove half of the sessions.
        {
            let mut sessions = manager.active_sessions.lock();
            for i in 0..5 {
                sessions.remove(&format!("session-{}", i));
            }
        }

        assert_eq!(manager.active_session_count(), 5);

        // Verify that removed sessions are inactive.
        for i in 0..5 {
            assert!(!manager.is_session_active(&format!("session-{}", i)));
        }

        // Verify that retained sessions are still active.
        for i in 5..10 {
            assert!(manager.is_session_active(&format!("session-{}", i)));
        }
    }

    #[test]
    fn test_session_reactivation() {
        let manager = SwLifecycleManager::new();

        // Add the session.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("reactivate-session".to_string());
        }

        assert!(manager.is_session_active("reactivate-session"));

        // Remove the session.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.remove("reactivate-session");
        }

        assert!(!manager.is_session_active("reactivate-session"));

        // Re-add the same session.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("reactivate-session".to_string());
        }

        assert!(manager.is_session_active("reactivate-session"));
    }

    #[test]
    fn test_wire_pool_integration() {
        let mut manager = SwLifecycleManager::new();
        let wire_pool = create_test_wire_pool();

        // Initially there is no wire pool.
        assert!(manager.wire_pool.is_none());

        // Set the wire pool.
        manager.set_wire_pool(wire_pool.clone());
        assert!(manager.wire_pool.is_some());

        // Verify that the wire pool is accessible.
        assert!(manager.wire_pool.is_some());
    }

    #[test]
    fn test_concurrent_session_operations() {
        let manager = SwLifecycleManager::new();

        // Simulate concurrent session additions.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("concurrent-1".to_string());
            sessions.insert("concurrent-2".to_string());
            sessions.insert("concurrent-3".to_string());
        }

        // Simulate concurrent reads.
        assert!(manager.is_session_active("concurrent-1"));
        assert!(manager.is_session_active("concurrent-2"));
        assert!(manager.is_session_active("concurrent-3"));

        // Simulate concurrent removals.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.remove("concurrent-2");
        }

        assert!(manager.is_session_active("concurrent-1"));
        assert!(!manager.is_session_active("concurrent-2"));
        assert!(manager.is_session_active("concurrent-3"));
    }

    #[test]
    fn test_empty_session_id_handling() {
        let manager = SwLifecycleManager::new();

        // Try querying an empty `session_id`.
        assert!(!manager.is_session_active(""));

        // Try manually inserting an empty `session_id`, even though `handle_dom_ready` rejects it.
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("".to_string());
        }

        assert!(manager.is_session_active(""));
        assert_eq!(manager.active_session_count(), 1);
    }
}
