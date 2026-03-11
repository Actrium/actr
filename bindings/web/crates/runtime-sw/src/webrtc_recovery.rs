//! WebRTC 连接恢复管理
//!
//! 负责在 DOM 重启后重建 WebRTC 连接

use crate::{PeerTransport, WebResult, WirePool};
use actr_web_common::ConnType;
use std::sync::Arc;
use web_sys::MessagePort;

/// WebRTC 恢复管理器
pub struct WebRtcRecoveryManager {
    /// WirePool 引用
    wire_pool: Arc<WirePool>,

    /// Transport Manager 引用（可选）
    transport_manager: Option<Arc<PeerTransport>>,
}

impl WebRtcRecoveryManager {
    /// 创建新的恢复管理器
    pub fn new(wire_pool: Arc<WirePool>) -> Self {
        log::info!("[WebRtcRecovery] Creating recovery manager");

        Self {
            wire_pool,
            transport_manager: None,
        }
    }

    /// 设置 Transport Manager 引用
    pub fn with_transport_manager(mut self, manager: Arc<PeerTransport>) -> Self {
        self.transport_manager = Some(manager);
        self
    }

    /// 处理 DOM 重启事件
    ///
    /// 当 DOM 发送 "DOM_READY" 消息时调用
    pub async fn handle_dom_restart(&self, session_id: String) -> WebResult<()> {
        log::info!(
            "[WebRtcRecovery] Handling DOM restart: session_id={}",
            session_id
        );

        // 1. 清理所有 WebRTC 连接
        self.cleanup_stale_connections();

        // 2. 请求 DOM 重新建立 WebRTC
        // 注意：实际的重建请求需要通过控制通道发送到 DOM
        // 这里仅记录日志，实际实现需要与 DOM 通信机制配合
        log::info!("[WebRtcRecovery] WebRTC connections cleaned, waiting for DOM to rebuild");

        Ok(())
    }

    /// 清理失效的连接
    fn cleanup_stale_connections(&self) {
        log::info!("[WebRtcRecovery] Cleaning up stale WebRTC connections");

        // 移除 WebRTC 连接
        self.wire_pool.remove_connection(ConnType::WebRTC);

        log::info!("[WebRtcRecovery] Stale connections removed");
    }

    /// 接收新的 MessagePort
    ///
    /// DOM 重建 WebRTC 后，会发送新的 MessagePort 到 SW
    pub fn register_new_port(&self, peer_id: String, port: MessagePort) -> WebResult<()> {
        log::info!(
            "[WebRtcRecovery] Registering new MessagePort for peer: {}",
            peer_id
        );

        // 创建新的 WebRTC 连接
        use crate::transport::WebRtcConnection;

        let mut rtc_conn = WebRtcConnection::new(peer_id.clone());
        rtc_conn.set_datachannel_port(port);

        // 重新添加到 WirePool
        use crate::transport::WireHandle;
        self.wire_pool.reconnect(WireHandle::WebRTC(rtc_conn));

        log::info!(
            "[WebRtcRecovery] WebRTC connection rebuilt successfully for peer: {}",
            peer_id
        );

        Ok(())
    }

    /// 请求 DOM 重建 WebRTC
    ///
    /// 向 DOM 发送重建请求消息
    async fn request_webrtc_rebuild(&self) -> WebResult<()> {
        log::info!("[WebRtcRecovery] Requesting WebRTC rebuild from DOM");

        // TODO: 实现控制消息发送
        // 需要一个专门的控制通道（不同于 DataLane）
        // 可以通过以下方式：
        // 1. 使用独立的 MessagePort（控制端口）
        // 2. 使用 ServiceWorker.postMessage 广播
        // 3. 通过现有的 DOM lane 发送特殊控制消息

        log::warn!("[WebRtcRecovery] Control channel not implemented yet");

        Ok(())
    }

    /// 检查恢复状态
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

    /// 获取 WirePool 引用
    pub fn wire_pool(&self) -> &Arc<WirePool> {
        &self.wire_pool
    }
}

/// 恢复状态
#[derive(Debug, Clone)]
pub struct RecoveryStatus {
    /// WebRTC 是否已连接
    pub webrtc_connected: bool,

    /// WebSocket 是否已连接
    pub websocket_connected: bool,

    /// 是否需要恢复
    pub needs_recovery: bool,
}

impl RecoveryStatus {
    /// 是否健康
    pub fn is_healthy(&self) -> bool {
        self.webrtc_connected || self.websocket_connected
    }

    /// 是否完全恢复
    pub fn is_fully_recovered(&self) -> bool {
        self.webrtc_connected && self.websocket_connected
    }
}

#[cfg(test)]
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
        // 创建成功即可
    }

    #[test]
    fn test_recovery_manager_with_transport_manager() {
        let wire_pool = create_test_wire_pool();
        let wire_builder = Arc::new(crate::transport::WebWireBuilder::new());
        let transport_manager = Arc::new(PeerTransport::new(
            "test-sw".to_string(),
            wire_builder,
        ));

        let manager =
            WebRtcRecoveryManager::new(wire_pool).with_transport_manager(transport_manager);

        // 验证可以正确设置 transport manager
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
        // 需要恢复：WebRTC 断开
        let status = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };

        assert_eq!(status.needs_recovery, true);

        // 不需要恢复：两者都连接
        let status2 = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: true,
            needs_recovery: false,
        };

        assert_eq!(status2.needs_recovery, false);
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

        // 验证可以获取 WirePool 引用
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
        // 测试所有关键组合
        let combinations = vec![
            (false, false, true), // 全部断开
            (false, true, true),  // 仅 WebSocket
            (true, false, false), // 仅 WebRTC
            (true, true, false),  // 全部连接
        ];

        for (webrtc, websocket, recovery) in combinations {
            let status = RecoveryStatus {
                webrtc_connected: webrtc,
                websocket_connected: websocket,
                needs_recovery: recovery,
            };

            // 验证逻辑一致性
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

        // 验证初始状态
        assert!(manager.transport_manager.is_none());
    }

    #[test]
    fn test_with_transport_manager_builder_pattern() {
        let wire_pool = create_test_wire_pool();
        let wire_builder = Arc::new(crate::transport::WebWireBuilder::new());
        let transport_manager = Arc::new(PeerTransport::new(
            "test-sw".to_string(),
            wire_builder,
        ));

        // 使用 builder 模式
        let manager =
            WebRtcRecoveryManager::new(wire_pool.clone()).with_transport_manager(transport_manager);

        assert!(manager.transport_manager.is_some());

        // 验证可以获取 wire_pool
        let pool_ref = manager.wire_pool();
        assert!(Arc::ptr_eq(pool_ref, &wire_pool));
    }

    #[test]
    fn test_recovery_status_needs_recovery_logic() {
        // WebRTC 断开时需要恢复
        let status1 = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };
        assert!(status1.needs_recovery);

        // 全部连接时不需要恢复
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

        // 验证 clone 后的相等性
        assert_eq!(status1.webrtc_connected, status2.webrtc_connected);
        assert_eq!(status1.websocket_connected, status2.websocket_connected);
        assert_eq!(status1.needs_recovery, status2.needs_recovery);
    }

    #[test]
    fn test_cleanup_stale_connections() {
        let wire_pool = create_test_wire_pool();
        let manager = WebRtcRecoveryManager::new(wire_pool.clone());

        // 调用 cleanup_stale_connections（通过内部方法）
        // 这个测试验证方法不会 panic
        manager.cleanup_stale_connections();
    }

    #[test]
    fn test_recovery_manager_multiple_instances() {
        let wire_pool1 = create_test_wire_pool();
        let wire_pool2 = create_test_wire_pool();

        let manager1 = WebRtcRecoveryManager::new(wire_pool1.clone());
        let manager2 = WebRtcRecoveryManager::new(wire_pool2.clone());

        // 验证每个 manager 有独立的 wire_pool
        assert!(!Arc::ptr_eq(manager1.wire_pool(), manager2.wire_pool()));
    }

    #[test]
    fn test_recovery_status_partial_recovery_scenarios() {
        // 场景 1：WebRTC 恢复中，WebSocket 已连接
        let status = RecoveryStatus {
            webrtc_connected: false,
            websocket_connected: true,
            needs_recovery: true,
        };
        assert!(status.is_healthy()); // 有一个连接就是健康的
        assert!(!status.is_fully_recovered()); // 但未完全恢复
        assert!(status.needs_recovery);

        // 场景 2：WebSocket 恢复中，WebRTC 已连接
        let status2 = RecoveryStatus {
            webrtc_connected: true,
            websocket_connected: false,
            needs_recovery: false, // 可能不需要恢复，因为 WebRTC 是主要连接
        };
        assert!(status2.is_healthy());
        assert!(!status2.is_fully_recovered());
    }

    #[test]
    fn test_recovery_manager_wire_pool_access() {
        let wire_pool = create_test_wire_pool();
        let manager = WebRtcRecoveryManager::new(wire_pool.clone());

        // 验证可以访问 wire_pool
        let pool_ref = manager.wire_pool();
        assert!(Arc::ptr_eq(pool_ref, &wire_pool));

        // 多次访问应该返回相同引用
        let pool_ref2 = manager.wire_pool();
        assert!(Arc::ptr_eq(pool_ref, pool_ref2));
    }
}
