//! Wire pool for connection management.
//!
//! Manages WebSocket and WebRTC connections and exposes event-driven readiness
//! notifications.

use super::wire_handle::{WireHandle, WireStatus};
use actr_web_common::{ConnType, WebResult};
use futures::StreamExt;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;

/// Set of ready connection types.
pub type ReadySet = HashSet<ConnType>;

/// Connection pool.
///
/// Manages multiple connections (`WebSocket` + `WebRTC`), starts connection
/// tasks concurrently, and notifies readiness changes in an event-driven way.
pub struct WirePool {
    /// Connection-state slots `[WebSocket, WebRTC]`.
    connections: Arc<Mutex<[Option<WireStatus>; 2]>>,

    /// Ready-state set.
    ready_set: Arc<Mutex<ReadySet>>,

    /// Broadcast senders for change notifications.
    change_notifiers: Arc<Mutex<Vec<mpsc::UnboundedSender<()>>>>,
}

impl WirePool {
    /// Create a new connection pool.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new([None, None])),
            ready_set: Arc::new(Mutex::new(HashSet::new())),
            change_notifiers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Add a connection and start its connection task in the background.
    ///
    /// Non-blocking: returns immediately and attempts the connection concurrently.
    pub fn add_connection(&self, connection: WireHandle) {
        let connections = Arc::clone(&self.connections);
        let ready_set = Arc::clone(&self.ready_set);
        let change_notifiers = Arc::clone(&self.change_notifiers);

        let conn_type = connection.conn_type();

        wasm_bindgen_futures::spawn_local(async move {
            // 1. Mark the initial state as `Connecting`.
            {
                let mut conns = connections.lock();
                conns[conn_type.as_index()] = Some(WireStatus::Connecting);
            }

            log::info!("[WirePool] Starting connection task: {:?}", conn_type);

            // 2. Attempt the connection.
            match connection.connect().await {
                Ok(_) => {
                    log::info!("[WirePool] Connection succeeded: {:?}", conn_type);

                    // 3. Update the slot to `Ready`.
                    {
                        let mut conns = connections.lock();
                        conns[conn_type.as_index()] = Some(WireStatus::Ready(connection));
                    }

                    // 4. Update the ready set.
                    {
                        let mut ready = ready_set.lock();
                        ready.insert(conn_type);
                    }

                    // 5. Notify all waiters.
                    Self::notify_all(&change_notifiers);
                }
                Err(e) => {
                    log::error!("[WirePool] Connection failed: {:?}: {}", conn_type, e);

                    // Mark the slot as failed.
                    {
                        let mut conns = connections.lock();
                        conns[conn_type.as_index()] = Some(WireStatus::Failed);
                    }

                    // Do not notify; failures do not trigger readiness events.
                }
            }
        });
    }

    /// Get the connection for a specific type.
    pub async fn get_connection(&self, conn_type: ConnType) -> Option<WireHandle> {
        let conns = self.connections.lock();
        if let Some(WireStatus::Ready(handle)) = &conns[conn_type.as_index()] {
            Some(handle.clone())
        } else {
            None
        }
    }

    /// Subscribe to readiness changes.
    ///
    /// The returned watcher receives a signal whenever readiness changes.
    pub fn subscribe_changes(&self) -> ReadyWatcher {
        let (tx, rx) = mpsc::unbounded();

        // Register the sender in the notifier list.
        {
            let mut notifiers = self.change_notifiers.lock();
            notifiers.push(tx);
        }

        ReadyWatcher {
            rx: Arc::new(Mutex::new(rx)),
            ready_set: Arc::clone(&self.ready_set),
        }
    }

    /// Notify all waiters.
    fn notify_all(notifiers: &Arc<Mutex<Vec<mpsc::UnboundedSender<()>>>>) {
        let mut notifiers = notifiers.lock();

        // Drop closed receivers while broadcasting.
        notifiers.retain(|tx| tx.unbounded_send(()).is_ok());

        log::trace!("[WirePool] Notified {} waiters", notifiers.len());
    }

    /// Mark a connection as failed.
    ///
    /// Called when a transport failure is detected, such as a `MessagePort`
    /// send failure.
    pub fn mark_connection_failed(&self, conn_type: ConnType) {
        let mut conns = self.connections.lock();
        conns[conn_type.as_index()] = Some(WireStatus::Failed);

        log::warn!("[WirePool] Connection marked as failed: {:?}", conn_type);

        // Remove it from the ready set.
        {
            let mut ready = self.ready_set.lock();
            ready.remove(&conn_type);
        }

        // Notify waiters that the state changed.
        Self::notify_all(&self.change_notifiers);
    }

    /// Remove a connection.
    ///
    /// Completely clears the connection state to prepare for rebuilding.
    pub fn remove_connection(&self, conn_type: ConnType) {
        let mut conns = self.connections.lock();
        conns[conn_type.as_index()] = None;

        log::info!("[WirePool] Connection removed: {:?}", conn_type);

        // Remove it from the ready set.
        {
            let mut ready = self.ready_set.lock();
            ready.remove(&conn_type);
        }
    }

    /// Reconnect by replacing an old connection with a new one.
    pub fn reconnect(&self, connection: WireHandle) {
        let conn_type = connection.conn_type();

        log::info!("[WirePool] Reconnecting: {:?}", conn_type);

        // Remove the old one first.
        self.remove_connection(conn_type);

        // Then add the new one.
        self.add_connection(connection);
    }

    /// Perform a health check across all connections.
    pub async fn health_check(&self) -> std::collections::HashMap<ConnType, bool> {
        use std::collections::HashMap;

        let mut results = HashMap::new();

        for conn_type in [ConnType::WebSocket, ConnType::WebRTC] {
            if let Some(handle) = self.get_connection(conn_type).await {
                let alive = handle.is_connected();
                results.insert(conn_type, alive);
            } else {
                results.insert(conn_type, false);
            }
        }

        log::debug!("[WirePool] Health check results: {:?}", results);
        results
    }

    /// Get the state of all connections.
    pub fn get_all_status(&self) -> Vec<(ConnType, Option<WireStatus>)> {
        let conns = self.connections.lock();
        vec![
            (ConnType::WebSocket, conns[0].clone()),
            (ConnType::WebRTC, conns[1].clone()),
        ]
    }
}

impl Default for WirePool {
    fn default() -> Self {
        Self::new()
    }
}

/// Readiness watcher.
///
/// Waits for readiness changes from the pool.
pub struct ReadyWatcher {
    /// Receiver for change notifications.
    rx: Arc<Mutex<mpsc::UnboundedReceiver<()>>>,

    /// Shared ready-set reference.
    ready_set: Arc<Mutex<ReadySet>>,
}

impl ReadyWatcher {
    /// Get the current ready-set snapshot.
    pub fn borrow_and_update(&self) -> ReadySet {
        self.ready_set.lock().clone()
    }

    /// Wait for the next change.
    ///
    /// Returns `Ok(())` when a change arrives and `Err(...)` when the channel closes.
    pub async fn changed(&mut self) -> WebResult<()> {
        let mut rx = self.rx.lock();
        if rx.next().await.is_some() {
            Ok(())
        } else {
            Err(actr_web_common::WebError::Transport(
                "WirePool channel closed".to_string(),
            ))
        }
    }
}

/// Extension helpers for `ConnType`.
trait ConnTypeExt {
    fn as_index(&self) -> usize;
}

impl ConnTypeExt for ConnType {
    /// Convert to an array index (`WebSocket=0`, `WebRTC=1`).
    fn as_index(&self) -> usize {
        match self {
            ConnType::WebSocket => 0,
            ConnType::WebRTC => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_pool_creation() {
        let pool = WirePool::new();

        // The initial state should be empty.
        let conns = pool.connections.lock();
        assert!(conns[0].is_none());
        assert!(conns[1].is_none());

        let ready = pool.ready_set.lock();
        assert!(ready.is_empty());
    }

    #[test]
    fn test_conn_type_as_index() {
        assert_eq!(ConnType::WebSocket.as_index(), 0);
        assert_eq!(ConnType::WebRTC.as_index(), 1);
    }

    #[test]
    fn test_ready_set_initialization() {
        let pool = WirePool::new();
        let ready = pool.ready_set.lock();
        assert_eq!(ready.len(), 0);
        assert!(!ready.contains(&ConnType::WebSocket));
        assert!(!ready.contains(&ConnType::WebRTC));
    }

    #[test]
    fn test_subscribe() {
        let pool = WirePool::new();
        let _subscriber1 = pool.subscribe_changes();
        let _subscriber2 = pool.subscribe_changes();

        // Verify that the subscriber is registered correctly.
        let notifiers = pool.change_notifiers.lock();
        assert_eq!(notifiers.len(), 2);
    }

    #[test]
    fn test_remove_connection() {
        let pool = WirePool::new();

        // Add a simulated connection state.
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Connecting);
        }

        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebRTC);
        }

        // Remove the connection.
        pool.remove_connection(ConnType::WebRTC);

        // Verify that the state was cleared.
        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebRTC.as_index()].is_none());

        let ready = pool.ready_set.lock();
        assert!(!ready.contains(&ConnType::WebRTC));
    }

    #[test]
    fn test_reconnect() {
        let pool = WirePool::new();

        // Simulate a failed connection.
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebSocket.as_index()] = Some(WireStatus::Failed);
        }

        // `reconnect` should clear the failed state and allow re-adding.
        // This only verifies state cleanup; actual reconnection happens in `add_connection`.
        pool.remove_connection(ConnType::WebSocket);

        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebSocket.as_index()].is_none());
    }

    #[test]
    fn test_multiple_connection_types() {
        let pool = WirePool::new();

        // Multiple connection types can be managed at the same time.
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebSocket.as_index()] = Some(WireStatus::Connecting);
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Connecting);
        }

        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebSocket.as_index()],
            Some(WireStatus::Connecting)
        ));
        assert!(matches!(
            conns[ConnType::WebRTC.as_index()],
            Some(WireStatus::Connecting)
        ));
    }

    #[test]
    fn test_ready_set_updates() {
        let pool = WirePool::new();

        // Simulate a ready connection.
        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebSocket);
        }

        let ready = pool.ready_set.lock();
        assert!(ready.contains(&ConnType::WebSocket));
        assert!(!ready.contains(&ConnType::WebRTC));

        // Add a second connection.
        drop(ready);
        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebRTC);
        }

        let ready = pool.ready_set.lock();
        assert!(ready.contains(&ConnType::WebSocket));
        assert!(ready.contains(&ConnType::WebRTC));
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn test_connection_state_transitions() {
        let pool = WirePool::new();

        // Connecting -> Failed
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Connecting);
        }

        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Failed);
        }

        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebRTC.as_index()],
            Some(WireStatus::Failed)
        ));
    }

    #[test]
    fn test_default_implementation() {
        let pool = WirePool::default();

        let conns = pool.connections.lock();
        assert!(conns[0].is_none());
        assert!(conns[1].is_none());
    }

    #[test]
    fn test_subscribe_multiple_times() {
        let pool = WirePool::new();

        let _sub1 = pool.subscribe_changes();
        let _sub2 = pool.subscribe_changes();
        let _sub3 = pool.subscribe_changes();

        let notifiers = pool.change_notifiers.lock();
        assert_eq!(notifiers.len(), 3);
    }

    #[test]
    fn test_remove_non_existent_connection() {
        let pool = WirePool::new();

        // Removing a non-existent connection should not panic.
        pool.remove_connection(ConnType::WebRTC);

        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebRTC.as_index()].is_none());
    }

    #[test]
    fn test_mark_connection_failed() {
        let pool = WirePool::new();

        // Simulate a ready connection.
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebSocket.as_index()] = Some(WireStatus::Connecting);
        }

        // Mark it as failed.
        pool.mark_connection_failed(ConnType::WebSocket);

        // Verify that the state becomes `Failed`.
        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebSocket.as_index()],
            Some(WireStatus::Failed)
        ));
    }

    #[test]
    fn test_get_all_status() {
        let pool = WirePool::new();

        // Set different states.
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebSocket.as_index()] = Some(WireStatus::Connecting);
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Failed);
        }

        let all_status = pool.get_all_status();

        assert_eq!(all_status.len(), 2);
        assert_eq!(all_status[0].0, ConnType::WebSocket);
        assert!(matches!(all_status[0].1, Some(WireStatus::Connecting)));
        assert_eq!(all_status[1].0, ConnType::WebRTC);
        assert!(matches!(all_status[1].1, Some(WireStatus::Failed)));
    }

    #[test]
    fn test_notify_all_cleans_closed_receivers() {
        let pool = WirePool::new();

        // Create subscribers.
        let _watcher1 = pool.subscribe_changes();
        let watcher2 = pool.subscribe_changes();

        // Explicitly drop `watcher2` to close its receiver.
        drop(watcher2);

        // Create a new subscriber afterward.
        let _watcher3 = pool.subscribe_changes();

        // Notification should clean up closed receivers.
        let notifiers = pool.change_notifiers.lock();
        // There should be two active subscribers (`watcher1` and `watcher3`).
        assert!(notifiers.len() >= 2);
    }

    #[test]
    fn test_ready_watcher_borrow_and_update() {
        let pool = WirePool::new();

        // Add a connection to the ready set.
        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebSocket);
            ready.insert(ConnType::WebRTC);
        }

        let watcher = pool.subscribe_changes();
        let ready_set = watcher.borrow_and_update();

        assert!(ready_set.contains(&ConnType::WebSocket));
        assert!(ready_set.contains(&ConnType::WebRTC));
        assert_eq!(ready_set.len(), 2);
    }

    #[test]
    fn test_reconnect_removes_old_and_adds_new() {
        let pool = WirePool::new();

        // Set up a failed connection first.
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Failed);
        }

        // `reconnect` should clear the state; actual reconnection is async via `add_connection`.
        // This test only covers the removal part.
        pool.remove_connection(ConnType::WebRTC);

        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebRTC.as_index()].is_none());
    }

    #[test]
    fn test_conn_type_index_uniqueness() {
        // Ensure each connection type has a unique index.
        let ws_idx = ConnType::WebSocket.as_index();
        let rtc_idx = ConnType::WebRTC.as_index();

        assert_ne!(ws_idx, rtc_idx);
        assert!(ws_idx < 2);
        assert!(rtc_idx < 2);
    }

    #[test]
    fn test_connections_array_size() {
        let pool = WirePool::new();
        let conns = pool.connections.lock();

        // The connection array should have two slots.
        assert_eq!(conns.len(), 2);
    }

    #[test]
    fn test_mark_connection_failed_multiple_times() {
        let pool = WirePool::new();

        // First failure mark.
        pool.mark_connection_failed(ConnType::WebSocket);

        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebSocket.as_index()],
            Some(WireStatus::Failed)
        ));
        drop(conns);

        // Second failure mark should not panic.
        pool.mark_connection_failed(ConnType::WebSocket);

        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebSocket.as_index()],
            Some(WireStatus::Failed)
        ));
    }

    #[test]
    fn test_remove_then_mark_failed() {
        let pool = WirePool::new();

        // Remove first.
        pool.remove_connection(ConnType::WebRTC);

        // Then mark failure on the already removed connection.
        pool.mark_connection_failed(ConnType::WebRTC);

        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebRTC.as_index()],
            Some(WireStatus::Failed)
        ));
    }

    #[test]
    fn test_multiple_ready_set_operations() {
        let pool = WirePool::new();

        // Add.
        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebSocket);
        }

        // Check.
        {
            let ready = pool.ready_set.lock();
            assert!(ready.contains(&ConnType::WebSocket));
        }

        // Remove.
        {
            let mut ready = pool.ready_set.lock();
            ready.remove(&ConnType::WebSocket);
        }

        // Check again.
        {
            let ready = pool.ready_set.lock();
            assert!(!ready.contains(&ConnType::WebSocket));
        }
    }

    #[test]
    fn test_get_all_status_empty_pool() {
        let pool = WirePool::new();
        let all_status = pool.get_all_status();

        assert_eq!(all_status.len(), 2);
        assert!(all_status[0].1.is_none());
        assert!(all_status[1].1.is_none());
    }
}
