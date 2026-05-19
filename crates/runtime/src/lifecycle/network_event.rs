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
//! │    └─► Ensure signaling + WebRTC recovery               │
//! │  • process_network_lost()                               │
//! │    └─► Clear pending + disconnect                       │
//! │  • process_network_type_changed()                       │
//! │    └─► Ensure signaling + WebRTC recovery               │
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
use tokio_util::sync::CancellationToken;

const NETWORK_EVENT_SETTLE_WINDOW: Duration = Duration::from_millis(400);

/// 网络事件类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkEvent {
    /// 网络可用（从断网恢复）
    Available,

    /// 网络丢失（断网）
    Lost,

    /// 网络类型变化（WiFi ↔ Cellular）
    TypeChanged { is_wifi: bool, is_cellular: bool },

    /// 主动清理所有连接
    ///
    /// 用于应用生命周期管理场景：
    /// - 应用进入后台
    /// - 用户主动登出
    /// - 应用即将退出
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

/// 网络事件处理结果
#[derive(Debug, Clone)]
pub struct NetworkEventResult {
    /// 事件类型
    pub event: NetworkEvent,

    /// 处理是否成功
    pub success: bool,

    /// 错误信息（如果失败）
    pub error: Option<String>,

    /// 处理耗时（毫秒）
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

/// 网络事件处理器 Trait
///
/// 定义网络事件的处理逻辑，可由用户自定义实现
#[async_trait::async_trait]
pub trait NetworkEventProcessor: Send + Sync {
    /// 处理网络可用事件
    ///
    /// # Returns
    /// - `Ok(())`: 处理成功
    /// - `Err(String)`: 处理失败，包含错误信息
    async fn process_network_available(&self) -> Result<(), String>;

    /// 处理网络丢失事件
    ///
    /// # Returns
    /// - `Ok(())`: 处理成功
    /// - `Err(String)`: 处理失败，包含错误信息
    async fn process_network_lost(&self) -> Result<(), String>;

    /// 处理网络类型变化事件
    ///
    /// # Returns
    /// - `Ok(())`: 处理成功
    /// - `Err(String)`: 处理失败，包含错误信息
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String>;

    /// 主动清理所有连接
    ///
    /// 此方法用于主动清理所有网络连接，适用于以下场景：
    /// - 应用进入后台（iOS/Android）
    /// - 用户主动登出
    /// - 应用即将退出
    /// - 需要重置网络状态
    ///
    /// # FFI Binding 说明
    ///
    /// 此方法专门设计用于 FFI binding，允许上层平台代码（Swift/Kotlin）
    /// 通过统一的 `NetworkEventProcessor` 接口主动管理连接生命周期。
    ///
    /// # 与事件响应的区别
    ///
    /// - `process_network_lost()`: 被动响应网络断开事件
    /// - `cleanup_connections()`: 主动清理连接（不依赖网络事件）
    ///
    /// # Returns
    /// - `Ok(())`: 清理成功
    /// - `Err(String)`: 清理失败，包含错误信息
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

/// 防抖配置
#[derive(Debug, Clone)]
pub struct DebounceConfig {
    /// 防抖时间窗口（同一事件在此时间内重复触发会被忽略）
    pub window: Duration,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            // 默认 1 秒防抖窗口
            window: Duration::from_secs(2),
        }
    }
}

/// 防抖状态跟踪
#[derive(Debug)]
struct DebounceState {
    last_available: tokio::sync::Mutex<Option<Instant>>,
    last_lost: tokio::sync::Mutex<Option<Instant>>,
    last_type_changed: tokio::sync::Mutex<Option<Instant>>,
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

impl DebounceState {
    fn new() -> Self {
        Self {
            last_available: tokio::sync::Mutex::new(None),
            last_lost: tokio::sync::Mutex::new(None),
            last_type_changed: tokio::sync::Mutex::new(None),
        }
    }
}

/// 默认网络事件处理器实现
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

    /// 检查事件是否应该被防抖过滤
    ///
    /// # Returns
    /// - `true`: 事件应该被处理
    /// - `false`: 事件在防抖窗口内，应该被忽略
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
            // CleanupConnections 不进行防抖检查，主动清理总是立即执行
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
            coordinator.begin_network_recovery().await;
        }

        self.rebuild_signaling_once(reason).await?;

        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            tracing::info!("🧹 Clearing stale ICE restart attempts before recovery...");
            coordinator.clear_pending_restarts().await;
            tracing::info!("♻️ Triggering ICE restart for failed connections...");
            coordinator.retry_failed_connections().await;
        }

        Ok(())
    }

    async fn process_offline(&self) -> Result<(), String> {
        tracing::info!("📱 Processing: Network offline");

        if let Some(ref coordinator) = self.webrtc_coordinator {
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
    /// 处理网络可用事件
    async fn process_network_available(&self) -> Result<(), String> {
        // 防抖检查
        let should_process = self.should_process_event(&NetworkEvent::Available).await;
        if !should_process && self.signaling_client.is_connected() {
            return Ok(());
        }

        tracing::info!("📱 Processing: Network available");

        self.restore_signaling_and_webrtc("NetworkAvailable").await
    }

    /// 处理网络丢失事件
    async fn process_network_lost(&self) -> Result<(), String> {
        // 防抖检查
        if !self.should_process_event(&NetworkEvent::Lost).await {
            return Ok(());
        }

        self.process_offline().await
    }

    /// 处理网络类型变化事件
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        // 防抖检查
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

    /// 主动清理所有连接
    ///
    /// 与 `process_network_lost()` 的区别：
    /// - 不进行防抖检查（主动调用总是执行）
    /// - 适用于应用生命周期管理，而非网络事件响应
    async fn cleanup_connections(&self) -> Result<(), String> {
        tracing::info!("🧹 Manually cleaning up all connections...");

        // Step 1: 清理待处理的 ICE 重启尝试
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("♻️  Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;

            // Step 2: 关闭所有 WebRTC peer connections
            tracing::info!("🔻 Closing all WebRTC peer connections...");
            if let Err(e) = coordinator.close_all_peers().await {
                let err_msg = format!("Failed to close all peers: {}", e);
                tracing::warn!("⚠️  {}", err_msg);
                // 不返回错误，继续清理其他资源
            } else {
                tracing::info!("✅ All WebRTC peer connections closed");
            }
        }

        // Step 3: 主动断开 WebSocket
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            match self.signaling_client.disconnect().await {
                Ok(_) => {
                    tracing::info!("✅ WebSocket disconnected successfully");
                }
                Err(e) => {
                    let err_msg = format!("Failed to disconnect WebSocket: {}", e);
                    tracing::warn!("⚠️  {}", err_msg);
                    // 不返回错误，继续清理其他资源
                }
            }
        }

        tracing::info!("✅ Connection cleanup completed");

        // Step 4: 立即重新建立信令连接
        // 确保 App 回到前台后立即可用，不需要等待自动重连
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

    /// 主动清理所有连接
    ///
    /// 此方法用于主动清理所有网络连接，适用于以下场景：
    /// - 应用进入后台（iOS/Android）
    /// - 用户主动登出
    /// - 应用即将退出
    /// - 需要重置网络状态
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
