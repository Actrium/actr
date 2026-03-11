//! Network Event Handling Architecture
//!
//! This module defines the network event handling infrastructure.
//!
//! # Architecture Overview
//!
//! ```text
//!        ┌─────────────────────────────────────────────┐
//!        │ (FFI Path - Implemented)  (Actor Path - TODO)
//!        ▼                                             ▼
//! ┌──────────────────────────┐      ┌──────────────────────────┐
//! │ NetworkEventHandle       │      │ Direct Proto Message     │
//! │ • Platform FFI calls     │      │ • Actor call/tell        │
//! │ • Send via channel       │      │ • Send to actor mailbox  │
//! │ • Await result           │      │ • No handle needed       │
//! └────────┬─────────────────┘      └──────┬───────────────────┘
//!          │                               │
//!          └───────────────┬───────────────┘
//!                          │ Both trigger
//!                          ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  ActrNode::network_event_loop()                         │
//! │  • Receive event from channel (FFI path)                │
//! │  • Or handle message directly (Actor path - TODO)       │
//! │  • Delegate to NetworkEventProcessor                    │
//! │  • Send result back via channel                         │
//! └──────────────────────┬──────────────────────────────────┘
//!                        │ Delegate
//!                        ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  NetworkEventProcessor (Trait)                          │
//! │                                                          │
//! │  DefaultNetworkEventProcessor:                          │
//! │  • process_network_available()                          │
//! │    └─► Reconnect signaling + ICE restart                │
//! │  • process_network_lost()                               │
//! │    └─► Clear pending + disconnect                       │
//! │  • process_network_type_changed()                       │
//! │    └─► Disconnect + wait + reconnect                    │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Key Components
//!
//! - **NetworkEvent**: Event types (Available, Lost, TypeChanged)
//! - **NetworkEventResult**: Processing result with success/error/duration
//! - **NetworkEventProcessor**: Trait for custom event handling logic
//! - **DefaultNetworkEventProcessor**: Default implementation with signaling + WebRTC recovery
//!
//! # Usage Patterns
//!
//! ## 1. Platform FFI Call (Primary, Implemented)
//! ```ignore
//! // Platform layer calls NetworkEventHandle via FFI
//! let network_handle = system.create_network_event_handle();
//! let result = network_handle.handle_network_available().await?;
//! if result.success {
//!     println!("✅ Processed in {}ms", result.duration_ms);
//! }
//! ```
//!
//! ## 2. Actor Proto Message (Optional, TODO)
//! ```ignore
//! // TODO: actors send proto message directly (not yet implemented)
//! actor_ref.call(NetworkAvailableMessage).await?;
//! ```
//!
//! **Key Differences:**
//! - FFI path: Uses NetworkEventHandle + channel (implemented)
//! - Actor path: Direct proto message to mailbox (TODO, future enhancement)

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::wire::webrtc::{SignalingClient, coordinator::WebRtcCoordinator};

/// Network event type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkEvent {
    /// Network available (recovered from disconnection)
    Available,

    /// Network lost (disconnected)
    Lost,

    /// Network type changed (WiFi <-> Cellular)
    TypeChanged { is_wifi: bool, is_cellular: bool },

    /// Proactively clean up all connections
    ///
    /// Used for app lifecycle management scenarios:
    /// - App entering background
    /// - User actively logging out
    /// - App about to exit
    CleanupConnections,
}

/// Network event processing result
#[derive(Debug, Clone)]
pub struct NetworkEventResult {
    /// Event type
    pub event: NetworkEvent,

    /// Whether processing succeeded
    pub success: bool,

    /// Error message (if failed)
    pub error: Option<String>,

    /// Processing duration (milliseconds)
    pub duration_ms: u64,
}

impl NetworkEventResult {
    pub fn success(event: NetworkEvent, duration_ms: u64) -> Self {
        Self {
            event,
            success: true,
            error: None,
            duration_ms,
        }
    }

    pub fn failure(event: NetworkEvent, error: String, duration_ms: u64) -> Self {
        Self {
            event,
            success: false,
            error: Some(error),
            duration_ms,
        }
    }
}

/// Network event processor trait
///
/// Defines the processing logic for network events; can be custom-implemented by users
#[async_trait::async_trait]
pub trait NetworkEventProcessor: Send + Sync {
    /// Process network available event
    ///
    /// # Returns
    /// - `Ok(())`: processing succeeded
    /// - `Err(String)`: processing failed, contains error message
    async fn process_network_available(&self) -> Result<(), String>;

    /// Process network lost event
    ///
    /// # Returns
    /// - `Ok(())`: processing succeeded
    /// - `Err(String)`: processing failed, contains error message
    async fn process_network_lost(&self) -> Result<(), String>;

    /// Process network type changed event
    ///
    /// # Returns
    /// - `Ok(())`: processing succeeded
    /// - `Err(String)`: processing failed, contains error message
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String>;

    /// Proactively clean up all connections
    ///
    /// This method proactively cleans up all network connections. Applicable scenarios:
    /// - App entering background (iOS/Android)
    /// - User actively logging out
    /// - App about to exit
    /// - Need to reset network state
    ///
    /// # FFI Binding Note
    ///
    /// This method is specifically designed for FFI bindings, allowing upper-layer
    /// platform code (Swift/Kotlin) to proactively manage connection lifecycle
    /// through the unified `NetworkEventProcessor` interface.
    ///
    /// # Difference from Event Response
    ///
    /// - `process_network_lost()`: passively responds to network disconnection events
    /// - `cleanup_connections()`: proactively cleans up connections (independent of network events)
    ///
    /// # Returns
    /// - `Ok(())`: cleanup succeeded
    /// - `Err(String)`: cleanup failed, contains error message
    async fn cleanup_connections(&self) -> Result<(), String>;
}

/// Debounce configuration
#[derive(Debug, Clone)]
pub struct DebounceConfig {
    /// Debounce time window (duplicate events within this window are ignored)
    pub window: Duration,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            // Default debounce window
            window: Duration::from_secs(2),
        }
    }
}

/// Debounce state tracking
#[derive(Debug)]
struct DebounceState {
    last_available: tokio::sync::Mutex<Option<Instant>>,
    last_lost: tokio::sync::Mutex<Option<Instant>>,
    last_type_changed: tokio::sync::Mutex<Option<Instant>>,
}

impl DebounceState {
    fn new() -> Self {
        Self {
            last_available: tokio::sync::Mutex::new(None),
            last_lost: tokio::sync::Mutex::new(None),
            last_type_changed: tokio::sync::Mutex::new(None),
        }
    }
}

/// Default network event processor implementation
pub struct DefaultNetworkEventProcessor {
    signaling_client: Arc<dyn SignalingClient>,
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    debounce_config: DebounceConfig,
    debounce_state: Arc<DebounceState>,
}

impl DefaultNetworkEventProcessor {
    pub fn new(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    ) -> Self {
        Self::new_with_debounce(
            signaling_client,
            webrtc_coordinator,
            DebounceConfig::default(),
        )
    }

    pub fn new_with_debounce(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        debounce_config: DebounceConfig,
    ) -> Self {
        Self {
            signaling_client,
            webrtc_coordinator,
            debounce_config,
            debounce_state: Arc::new(DebounceState::new()),
        }
    }

    /// Check whether an event should be filtered by debounce
    ///
    /// # Returns
    /// - `true`: the event should be processed
    /// - `false`: the event is within the debounce window and should be ignored
    async fn should_process_event(&self, event: &NetworkEvent) -> bool {
        let now = Instant::now();

        match event {
            NetworkEvent::Available => {
                let mut last = self.debounce_state.last_available.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network Available event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            NetworkEvent::Lost => {
                let mut last = self.debounce_state.last_lost.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network Lost event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            NetworkEvent::TypeChanged { .. } => {
                let mut last = self.debounce_state.last_type_changed.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network TypeChanged event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            // CleanupConnections skips debounce check; proactive cleanup always executes immediately
            NetworkEvent::CleanupConnections => {
                tracing::debug!(
                    "🧹 CleanupConnections event - no debouncing (always execute immediately)"
                );
                true
            }
        }
    }

    /// Internal reconnect method (no debounce check)
    ///
    /// Used in scenarios like `process_network_type_changed()` that must ensure reconnection.
    /// Differs from `process_network_available()`:
    /// - No debounce check (internal calls always execute)
    /// - Suitable for compound operations that have already passed debounce checks
    async fn reconnect_internal(&self) -> Result<(), String> {
        tracing::info!("🔄 Internal reconnect (bypassing debounce)");

        // Step 1: Force disconnect existing connections (avoid "zombie connections")
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting existing connection to ensure fresh state...");
            let _ = self.signaling_client.disconnect().await;
        }

        // Step 2: Wait briefly for network stabilization
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Step 3: Establish new WebSocket connection
        tracing::info!("🔄 Reconnecting WebSocket...");
        match self.signaling_client.connect().await {
            Ok(_) => {
                tracing::info!("✅ WebSocket reconnected successfully");
            }
            Err(e) => {
                let err_msg = format!("WebSocket reconnect failed: {}", e);
                tracing::error!("❌ {}", err_msg);
                return Err(err_msg);
            }
        }

        // Step 4: Trigger ICE restart (if WebRTC is initialized)
        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            tracing::info!("♻️ Triggering ICE restart for failed connections...");
            coordinator.retry_failed_connections().await;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for DefaultNetworkEventProcessor {
    /// Process network available event
    async fn process_network_available(&self) -> Result<(), String> {
        // Debounce check
        if !self.should_process_event(&NetworkEvent::Available).await {
            return Ok(());
        }

        tracing::info!("📱 Processing: Network available");

        // Step 1: Force disconnect existing connections (avoid "zombie connections")
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting existing connection to ensure fresh state...");
            let _ = self.signaling_client.disconnect().await;
        }

        // Step 2: Wait briefly for network stabilization
        // Note: this delay must be shorter than the debounce window, otherwise debounce becomes ineffective
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Step 3: Establish new WebSocket connection
        tracing::info!("🔄 Reconnecting WebSocket...");
        match self.signaling_client.connect().await {
            Ok(_) => {
                tracing::info!("✅ WebSocket reconnected successfully");
            }
            Err(e) => {
                let err_msg = format!("WebSocket reconnect failed: {}", e);
                tracing::error!("❌ {}", err_msg);
                return Err(err_msg);
            }
        }

        // Step 4: Trigger ICE restart (if WebRTC is initialized)
        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            tracing::info!("♻️ Triggering ICE restart for failed connections...");
            coordinator.retry_failed_connections().await;
        }

        Ok(())
    }

    /// Process network lost event
    async fn process_network_lost(&self) -> Result<(), String> {
        // Debounce check
        if !self.should_process_event(&NetworkEvent::Lost).await {
            return Ok(());
        }

        tracing::info!("📱 Processing: Network lost");

        // Step 1: Clear pending ICE restart attempts
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("🧹 Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        // Step 2: Proactively disconnect WebSocket
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            let _ = self.signaling_client.disconnect().await;
        }

        Ok(())
    }

    /// Process network type changed event
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        // Debounce check
        if !self
            .should_process_event(&NetworkEvent::TypeChanged {
                is_wifi,
                is_cellular,
            })
            .await
        {
            return Ok(());
        }

        tracing::info!(
            "📱 Processing: Network type changed (WiFi={}, Cellular={})",
            is_wifi,
            is_cellular
        );

        // Network type change usually implies IP address change
        // Treat as disconnect + recovery sequence

        // Step 1: Clean up existing connections
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("🧹 Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            let _ = self.signaling_client.disconnect().await;
        }

        // Step 2: Wait for network stabilization
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Step 3: Use internal reconnect method (bypassing debounce check)
        self.reconnect_internal().await?;

        Ok(())
    }

    /// Proactively clean up all connections
    ///
    /// Differs from `process_network_lost()`:
    /// - No debounce check (proactive calls always execute)
    /// - Intended for app lifecycle management, not network event response
    async fn cleanup_connections(&self) -> Result<(), String> {
        tracing::info!("🧹 Manually cleaning up all connections...");

        // Step 1: Clear pending ICE restart attempts
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("♻️  Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;

            // Step 2: Close all WebRTC peer connections
            tracing::info!("🔻 Closing all WebRTC peer connections...");
            if let Err(e) = coordinator.close_all_peers().await {
                let err_msg = format!("Failed to close all peers: {}", e);
                tracing::warn!("⚠️  {}", err_msg);
                // Do not fail the whole cleanup; continue releasing other resources.
            } else {
                tracing::info!("✅ All WebRTC peer connections closed");
            }
        }

        // Step 3: Proactively disconnect the WebSocket.
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            match self.signaling_client.disconnect().await {
                Ok(_) => {
                    tracing::info!("✅ WebSocket disconnected successfully");
                }
                Err(e) => {
                    let err_msg = format!("Failed to disconnect WebSocket: {}", e);
                    tracing::warn!("⚠️  {}", err_msg);
                    // Do not fail the whole cleanup; continue releasing other resources.
                }
            }
        }

        tracing::info!("✅ Connection cleanup completed");

        // Step 4: Re-establish signaling immediately.
        // This keeps the app usable as soon as it returns to the foreground.
        tracing::info!("🔌 Re-establishing signaling connection...");
        match self.signaling_client.connect().await {
            Ok(_) => {
                tracing::info!("✅ Signaling reconnected successfully after cleanup");
            }
            Err(e) => {
                let err_msg = format!("Failed to reconnect signaling after cleanup: {}", e);
                tracing::error!("❌ {}", err_msg);
                return Err(err_msg);
            }
        }

        tracing::info!("✅ Connection cleanup and reconnect completed");
        Ok(())
    }
}

/// Network Event Handle
///
/// Lightweight handle for sending network events and receiving processing results.
/// Created by `ActrSystem::create_network_event_handle()`.
pub struct NetworkEventHandle {
    /// Event sender (to ActrNode)
    event_tx: tokio::sync::mpsc::Sender<NetworkEvent>,

    /// Result receiver (from ActrNode)
    /// Wrapped in Arc<Mutex> to allow cloning
    result_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<NetworkEventResult>>>,
}

impl NetworkEventHandle {
    /// Create a new NetworkEventHandle
    pub fn new(
        event_tx: tokio::sync::mpsc::Sender<NetworkEvent>,
        result_rx: tokio::sync::mpsc::Receiver<NetworkEventResult>,
    ) -> Self {
        Self {
            event_tx,
            result_rx: Arc::new(tokio::sync::Mutex::new(result_rx)),
        }
    }

    /// Handle network available event
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn handle_network_available(&self) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::Available)
            .await
    }

    /// Handle network lost event
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn handle_network_lost(&self) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::Lost).await
    }

    /// Handle network type changed event
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn handle_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::TypeChanged {
            is_wifi,
            is_cellular,
        })
        .await
    }

    /// Proactively clean up all connections.
    ///
    /// Use this to proactively clean up all network connections in cases such as:
    /// - App entering the background (iOS/Android)
    /// - User logging out
    /// - App preparing to exit
    /// - Network state reset
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn cleanup_connections(&self) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::CleanupConnections)
            .await
    }

    /// Send event and await result (internal helper)
    async fn send_event_and_await_result(
        &self,
        event: NetworkEvent,
    ) -> Result<NetworkEventResult, String> {
        // Send event
        self.event_tx
            .send(event.clone())
            .await
            .map_err(|e| format!("Failed to send network event: {}", e))?;

        // Await result
        let mut rx = self.result_rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| "Failed to receive network event result".to_string())
    }
}

impl Clone for NetworkEventHandle {
    fn clone(&self) -> Self {
        Self {
            event_tx: self.event_tx.clone(),
            result_rx: self.result_rx.clone(),
        }
    }
}

/// Deduplicate network events by type while keeping the newest event of each type
/// and preserving original ordering.
///
/// # Algorithm
///
/// 1. Walk all events and group them by type.
/// 2. Keep only the last occurrence for each type.
/// 3. Sort by original index to preserve event ordering.
///
/// # Why deduplicate?
///
/// During foreground recovery or frequent network changes, the queue can fill up
/// with stale events. For a given network event type, only the newest state matters.
/// Different event types still represent distinct transitions and must be kept.
///
/// - `[Available, Lost, Available]` -> keeping only the last `Available` would lose the `Lost`
/// - `[Lost, Available, Lost]` -> keeping only the last `Lost` would lose the `Available`
///
/// Deduplicating by type ensures each kind of state transition is still processed.
pub fn deduplicate_network_events(events: Vec<NetworkEvent>) -> Vec<NetworkEvent> {
    use std::collections::HashMap;
    use std::mem::discriminant;

    if events.is_empty() {
        return vec![];
    }

    // Group by event type and keep the newest event for each discriminant.
    let mut latest_by_type: HashMap<std::mem::Discriminant<NetworkEvent>, (usize, NetworkEvent)> =
        HashMap::new();

    for (index, event) in events.into_iter().enumerate() {
        let event_discriminant = discriminant(&event);
        latest_by_type
            .entry(event_discriminant)
            .and_modify(|(idx, e)| {
                // Replace with the newer event.
                *idx = index;
                *e = event.clone();
            })
            .or_insert((index, event));
    }

    // Sort by original index to preserve temporal ordering.
    let mut deduplicated: Vec<_> = latest_by_type.into_values().collect();
    deduplicated.sort_by_key(|(index, _)| *index);

    // Extract the event payloads.
    deduplicated.into_iter().map(|(_, event)| event).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduplicate_empty() {
        let events = vec![];
        let result = deduplicate_network_events(events);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_deduplicate_single_event() {
        let events = vec![NetworkEvent::Available];
        let result = deduplicate_network_events(events);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], NetworkEvent::Available));
    }

    #[test]
    fn test_deduplicate_same_type_events() {
        let events = vec![
            NetworkEvent::Available,
            NetworkEvent::Available,
            NetworkEvent::Available,
        ];
        let result = deduplicate_network_events(events);

        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], NetworkEvent::Available));
    }

    #[test]
    fn test_deduplicate_different_type_events() {
        let events = vec![
            NetworkEvent::Available,
            NetworkEvent::Lost,
            NetworkEvent::TypeChanged {
                is_wifi: true,
                is_cellular: false,
            },
        ];
        let result = deduplicate_network_events(events);

        assert_eq!(result.len(), 3);
        assert!(matches!(result[0], NetworkEvent::Available));
        assert!(matches!(result[1], NetworkEvent::Lost));
        assert!(matches!(result[2], NetworkEvent::TypeChanged { .. }));
    }

    #[test]
    fn test_deduplicate_mixed_events() {
        let events = vec![
            NetworkEvent::Available, // #0
            NetworkEvent::Lost,      // #1
            NetworkEvent::Available, // #2 (replaces #0)
            NetworkEvent::TypeChanged {
                // #3
                is_wifi: true,
                is_cellular: false,
            },
            NetworkEvent::Lost,      // #4 (replaces #1)
            NetworkEvent::Available, // #5 (replaces #2)
        ];
        let result = deduplicate_network_events(events);

        // Expected result: TypeChanged (#3), Lost (#4), Available (#5).
        assert_eq!(result.len(), 3);

        // Verify order by original index.
        assert!(matches!(result[0], NetworkEvent::TypeChanged { .. }));
        assert!(matches!(result[1], NetworkEvent::Lost));
        assert!(matches!(result[2], NetworkEvent::Available));
    }

    #[test]
    fn test_deduplicate_preserves_order() {
        let events = vec![
            NetworkEvent::Lost, // #0
            NetworkEvent::TypeChanged {
                // #1
                is_wifi: true,
                is_cellular: false,
            },
            NetworkEvent::Available, // #2
            NetworkEvent::Lost,      // #3 (replaces #0)
        ];
        let result = deduplicate_network_events(events);

        // Expected result: TypeChanged (#1), Available (#2), Lost (#3).
        assert_eq!(result.len(), 3);

        // Verify order.
        assert!(matches!(result[0], NetworkEvent::TypeChanged { .. }));
        assert!(matches!(result[1], NetworkEvent::Available));
        assert!(matches!(result[2], NetworkEvent::Lost));
    }
}
