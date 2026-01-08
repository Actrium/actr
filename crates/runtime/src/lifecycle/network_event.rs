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
use std::time::Duration;

use crate::wire::webrtc::{SignalingClient, coordinator::WebRtcCoordinator};

/// 网络事件类型
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    /// 网络可用（从断网恢复）
    Available,

    /// 网络丢失（断网）
    Lost,

    /// 网络类型变化（WiFi ↔ Cellular）
    TypeChanged { is_wifi: bool, is_cellular: bool },
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
}

/// 默认网络事件处理器实现
pub struct DefaultNetworkEventProcessor {
    signaling_client: Arc<dyn SignalingClient>,
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
}

impl DefaultNetworkEventProcessor {
    pub fn new(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    ) -> Self {
        Self {
            signaling_client,
            webrtc_coordinator,
        }
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for DefaultNetworkEventProcessor {
    /// 处理网络可用事件
    async fn process_network_available(&self) -> Result<(), String> {
        tracing::info!("📱 Processing: Network available");

        // Step 1: 强制断开现有连接（避免"僵尸连接"）
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting existing connection to ensure fresh state...");
            let _ = self.signaling_client.disconnect().await;
        }

        // Step 2: 短暂延迟，让资源清理
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Step 3: 建立新的 WebSocket 连接
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

        // Step 4: 触发 ICE 重启（如果 WebRTC 已初始化）
        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            tracing::info!("♻️ Triggering ICE restart for failed connections...");
            coordinator.retry_failed_connections().await;
        }

        Ok(())
    }

    /// 处理网络丢失事件
    async fn process_network_lost(&self) -> Result<(), String> {
        tracing::info!("📱 Processing: Network lost");

        // Step 1: 清理待处理的 ICE 重启尝试
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("🧹 Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        // Step 2: 主动断开 WebSocket
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            let _ = self.signaling_client.disconnect().await;
        }

        Ok(())
    }

    /// 处理网络类型变化事件
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        tracing::info!(
            "📱 Processing: Network type changed (WiFi={}, Cellular={})",
            is_wifi,
            is_cellular
        );

        // 网络类型变化通常意味着 IP 地址变化
        // 视为断网 + 恢复序列

        // Step 1: 作为网络丢失处理
        self.process_network_lost().await?;

        // Step 2: 等待网络稳定
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Step 3: 作为网络恢复处理
        self.process_network_available().await?;

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
