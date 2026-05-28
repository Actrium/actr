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

use crate::wire::webrtc::{SignalingClient, WebRtcCoordinator};
use tokio_util::sync::CancellationToken;

const NETWORK_EVENT_SETTLE_WINDOW: Duration = Duration::from_millis(400);

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

/// Final action selected from a settled batch of network events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRecoveryAction {
    Noop,
    Offline,
    Restore,
    CleanupConnectionsCompat,
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

    /// Process the final action selected from a settled event batch.
    ///
    /// Custom processors can rely on the default mapping. The default runtime
    /// processor overrides this to bypass per-event debounce after reconciliation.
    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        match action {
            NetworkRecoveryAction::Noop => Ok(()),
            NetworkRecoveryAction::Offline => self.process_network_lost().await,
            NetworkRecoveryAction::Restore => self.process_network_available().await,
            NetworkRecoveryAction::CleanupConnectionsCompat => self.cleanup_connections().await,
        }
    }
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

#[derive(Debug)]
struct SignalingRecoveryState {
    connect_lock: tokio::sync::Mutex<()>,
    last_successful_connect: tokio::sync::Mutex<Option<Instant>>,
}

impl SignalingRecoveryState {
    fn new() -> Self {
        Self {
            connect_lock: tokio::sync::Mutex::new(()),
            last_successful_connect: tokio::sync::Mutex::new(None),
        }
    }
}

/// Default network event processor implementation
pub struct DefaultNetworkEventProcessor {
    signaling_client: Arc<dyn SignalingClient>,
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    debounce_config: DebounceConfig,
    debounce_state: Arc<DebounceState>,
    recovery_state: Arc<SignalingRecoveryState>,
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
            recovery_state: Arc::new(SignalingRecoveryState::new()),
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

    async fn ensure_signaling_connected_once(&self, reason: &str) -> Result<(), String> {
        let _guard = self.recovery_state.connect_lock.lock().await;

        if self.signaling_client.is_connected() {
            tracing::debug!(
                reason = reason,
                "Signaling already connected, skipping connect"
            );
            return Ok(());
        }

        let recently_connected = {
            let last = self.recovery_state.last_successful_connect.lock().await;
            last.map(|instant| instant.elapsed() < Duration::from_millis(1500))
                .unwrap_or(false)
        };
        if recently_connected && self.signaling_client.is_connected() {
            tracing::debug!(
                reason = reason,
                "Signaling recently connected, reusing connection"
            );
            return Ok(());
        }

        tracing::info!(reason = reason, "🔄 Connecting signaling");
        self.signaling_client.connect_once().await.map_err(|e| {
            let err_msg = format!("WebSocket connect failed: {}", e);
            tracing::error!("❌ {}", err_msg);
            err_msg
        })?;

        *self.recovery_state.last_successful_connect.lock().await = Some(Instant::now());
        tracing::info!(reason = reason, "✅ Signaling connected");
        Ok(())
    }

    async fn rebuild_signaling_once(&self, reason: &str) -> Result<(), String> {
        let _guard = self.recovery_state.connect_lock.lock().await;

        if self.signaling_client.is_connected() {
            tracing::info!(
                reason = reason,
                "🔌 Rebuilding signaling: disconnecting existing WebSocket"
            );
            if let Err(e) = self.signaling_client.disconnect().await {
                tracing::warn!(
                    reason = reason,
                    "⚠️ Failed to disconnect existing signaling before rebuild: {}",
                    e
                );
            }
        }

        tracing::info!(reason = reason, "🔄 Rebuilding signaling: connecting");
        self.signaling_client.connect_once().await.map_err(|e| {
            let err_msg = format!("WebSocket rebuild failed: {}", e);
            tracing::error!("❌ {}", err_msg);
            err_msg
        })?;

        *self.recovery_state.last_successful_connect.lock().await = Some(Instant::now());
        tracing::info!(reason = reason, "✅ Signaling rebuilt");
        Ok(())
    }

    async fn restore_signaling_and_webrtc(&self, reason: &str) -> Result<(), String> {
        if let Some(coordinator) = self.webrtc_coordinator.clone() {
            coordinator.begin_network_recovery(reason).await;
        }

        self.rebuild_signaling_once(reason).await?;

        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            tracing::info!("🧹 Clearing stale ICE restart attempts before recovery...");
            coordinator.clear_pending_restarts().await;
            tracing::info!("♻️ Triggering ICE restart for recovering connections...");
            coordinator.restart_network_recovery_connections().await;
        }

        Ok(())
    }

    async fn process_offline(&self) -> Result<(), String> {
        tracing::info!("📱 Processing: Network offline");

        if let Some(ref coordinator) = self.webrtc_coordinator {
            coordinator.begin_network_recovery("NetworkLost").await;
            tracing::info!("🧹 Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            let _ = self.signaling_client.disconnect().await;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for DefaultNetworkEventProcessor {
    /// Process network available event
    async fn process_network_available(&self) -> Result<(), String> {
        // Debounce check
        let should_process = self.should_process_event(&NetworkEvent::Available).await;
        if !should_process && self.signaling_client.is_connected() {
            return Ok(());
        }

        tracing::info!("📱 Processing: Network available");

        self.restore_signaling_and_webrtc("NetworkAvailable").await
    }

    /// Process network lost event
    async fn process_network_lost(&self) -> Result<(), String> {
        // Debounce check
        if !self.should_process_event(&NetworkEvent::Lost).await {
            return Ok(());
        }

        self.process_offline().await
    }

    /// Process network type changed event
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        // Debounce check
        let should_process = self
            .should_process_event(&NetworkEvent::TypeChanged {
                is_wifi,
                is_cellular,
            })
            .await;
        if !should_process && self.signaling_client.is_connected() {
            return Ok(());
        }

        tracing::info!(
            "📱 Processing: Network type changed (WiFi={}, Cellular={})",
            is_wifi,
            is_cellular
        );

        self.restore_signaling_and_webrtc("NetworkTypeChanged")
            .await
    }

    /// Proactively clean up all connections
    ///
    /// Differs from `process_network_lost()`:
    /// - No debounce check (proactive calls always execute)
    /// - Intended for app lifecycle management, not network event response
    async fn cleanup_connections(&self) -> Result<(), String> {
        let _cleanup_guard = self
            .webrtc_coordinator
            .as_ref()
            .map(|coordinator| coordinator.cleanup_guard());

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
        self.ensure_signaling_connected_once("CompatCleanupConnections")
            .await?;

        tracing::info!("✅ Connection cleanup and reconnect completed");
        Ok(())
    }

    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        match action {
            NetworkRecoveryAction::Noop => Ok(()),
            NetworkRecoveryAction::Offline => self.process_offline().await,
            NetworkRecoveryAction::Restore => {
                self.restore_signaling_and_webrtc("NetworkEventBatch").await
            }
            NetworkRecoveryAction::CleanupConnectionsCompat => self.cleanup_connections().await,
        }
    }
}

pub fn select_network_recovery_action(events: &[NetworkEvent]) -> NetworkRecoveryAction {
    let mut saw_cleanup_connections = false;
    let mut latest_state_action = NetworkRecoveryAction::Noop;

    for event in events {
        match event {
            NetworkEvent::CleanupConnections => saw_cleanup_connections = true,
            NetworkEvent::Available | NetworkEvent::TypeChanged { .. } => {
                latest_state_action = NetworkRecoveryAction::Restore
            }
            NetworkEvent::Lost => latest_state_action = NetworkRecoveryAction::Offline,
        }
    }

    if saw_cleanup_connections {
        NetworkRecoveryAction::CleanupConnectionsCompat
    } else {
        latest_state_action
    }
}

pub async fn process_network_event_batch(
    events: Vec<NetworkEvent>,
    processor: Arc<dyn NetworkEventProcessor>,
) -> Vec<NetworkEventResult> {
    if events.is_empty() {
        return Vec::new();
    }

    let action = select_network_recovery_action(&events);
    let start = Instant::now();

    let result = processor.process_network_recovery_action(action).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    events
        .into_iter()
        .map(|event| match &result {
            Ok(()) => NetworkEventResult::success(event, duration_ms),
            Err(e) => NetworkEventResult::failure(event, e.clone(), duration_ms),
        })
        .collect()
}

pub async fn run_network_event_reconciler(
    mut event_rx: tokio::sync::mpsc::Receiver<NetworkEvent>,
    result_tx: tokio::sync::mpsc::Sender<NetworkEventResult>,
    processor: Arc<dyn NetworkEventProcessor>,
    shutdown_token: CancellationToken,
) {
    tracing::info!("🔄 Network event reconciler started");

    loop {
        tokio::select! {
            Some(first_event) = event_rx.recv() => {
                let mut events = vec![first_event];
                let settle = tokio::time::sleep(NETWORK_EVENT_SETTLE_WINDOW);
                tokio::pin!(settle);

                loop {
                    tokio::select! {
                        Some(next_event) = event_rx.recv() => {
                            events.push(next_event);
                        }
                        _ = &mut settle => {
                            break;
                        }
                        _ = shutdown_token.cancelled() => {
                            tracing::info!("🛑 Network event reconciler shutting down");
                            return;
                        }
                        else => {
                            break;
                        }
                    }
                }

                while let Ok(next_event) = event_rx.try_recv() {
                    events.push(next_event);
                }

                let action = select_network_recovery_action(&events);
                tracing::info!(
                    event_count = events.len(),
                    action = ?action,
                    settle_window_ms = NETWORK_EVENT_SETTLE_WINDOW.as_millis() as u64,
                    "📱 Processing settled network event batch"
                );

                let results = process_network_event_batch(events, processor.clone()).await;

                for result in results {
                    if let Err(e) = result_tx.send(result).await {
                        tracing::warn!("Failed to send event result: {}", e);
                    }
                }
            }
            _ = shutdown_token.cancelled() => {
                tracing::info!("🛑 Network event reconciler shutting down");
                break;
            }
            else => break,
        }
    }
}

/// Network Event Handle
///
/// Lightweight handle for sending network events and receiving processing results.
/// Created before `ActrNode::start()` to bridge platform network events.
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
