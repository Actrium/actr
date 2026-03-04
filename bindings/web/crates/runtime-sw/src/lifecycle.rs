//! Service Worker 生命周期管理
//!
//! 负责监听 DOM 进程的生命周期事件，并协调资源清理和恢复

use crate::error_handler::get_global_error_handler;
use crate::{ConnType, WebError, WebResult, WirePool};
use actr_web_common::ControlMessage;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{ExtendableMessageEvent, ServiceWorkerGlobalScope};

/// Service Worker 生命周期管理器
pub struct SwLifecycleManager {
    /// 当前活跃的 DOM 会话集合
    active_sessions: Arc<Mutex<HashSet<String>>>,

    /// WirePool 引用（用于清理失效连接）
    wire_pool: Option<Arc<WirePool>>,
}

impl SwLifecycleManager {
    /// 创建新的生命周期管理器
    pub fn new() -> Self {
        log::info!("[SwLifecycle] Creating lifecycle manager");

        Self {
            active_sessions: Arc::new(Mutex::new(HashSet::new())),
            wire_pool: None,
        }
    }

    /// 设置 WirePool 引用
    ///
    /// 用于在 DOM 重启时清理失效的 WebRTC 连接
    pub fn set_wire_pool(&mut self, wire_pool: Arc<WirePool>) {
        log::info!("[SwLifecycle] WirePool registered");
        self.wire_pool = Some(wire_pool);
    }

    /// 初始化生命周期管理
    ///
    /// 设置全局消息监听器
    pub fn init(&self) -> WebResult<()> {
        log::info!("[SwLifecycle] Initializing lifecycle management");

        self.setup_message_listener()?;

        log::info!("[SwLifecycle] Lifecycle management initialized");
        Ok(())
    }

    /// 设置 Service Worker 全局消息监听器
    fn setup_message_listener(&self) -> WebResult<()> {
        let active_sessions = Arc::clone(&self.active_sessions);
        let wire_pool = self.wire_pool.clone();

        // 获取 ServiceWorkerGlobalScope
        let global = js_sys::global();
        let sw_global = global
            .dyn_into::<ServiceWorkerGlobalScope>()
            .map_err(|_| WebError::Internal("Not in Service Worker context".into()))?;

        // 创建消息处理回调
        let callback = Closure::wrap(Box::new(move |event: ExtendableMessageEvent| {
            let data = event.data();

            // 尝试作为序列化的 ControlMessage 处理（来自 DOM error_reporter）
            if data.dyn_ref::<js_sys::Object>().is_some() {
                // 尝试反序列化为 Vec<u8>（serde_wasm_bindgen 格式）
                if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(data.clone()) {
                    if let Ok(control_msg) = ControlMessage::deserialize(&bytes) {
                        match control_msg {
                            ControlMessage::ErrorReport(error_report) => {
                                log::debug!(
                                    "[SwLifecycle] Received error report via SW controller: {:?}",
                                    error_report.category
                                );

                                // 转发给全局错误处理器
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
                                // 其他 ControlMessage 类型，继续处理
                            }
                        }
                    }
                }
            }

            // 尝试作为普通对象处理（生命周期消息）
            if let Ok(data_obj) = data.dyn_into::<js_sys::Object>() {
                // 提取消息类型
                if let Ok(msg_type_js) = js_sys::Reflect::get(&data_obj, &"type".into()) {
                    if let Some(msg_type) = msg_type_js.as_string() {
                        // 提取 session_id
                        let session_id = if let Ok(session_id_js) =
                            js_sys::Reflect::get(&data_obj, &"session_id".into())
                        {
                            session_id_js.as_string().unwrap_or_default()
                        } else {
                            String::new()
                        };

                        // 处理不同类型的生命周期消息
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
                                // 忽略其他消息
                            }
                        }
                    }
                }
            }
        }) as Box<dyn FnMut(ExtendableMessageEvent)>);

        // 注册到 SW 的 message 事件
        sw_global
            .add_event_listener_with_callback("message", callback.as_ref().unchecked_ref())
            .map_err(|e| WebError::Internal(format!("Failed to add message listener: {:?}", e)))?;

        // 保持回调活跃
        callback.forget();

        log::info!("[SwLifecycle] Message listener registered");
        Ok(())
    }

    /// 处理 DOM_READY 消息
    ///
    /// DOM 进程重启后发送此消息
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

        // 添加到活跃会话集合
        {
            let mut sessions = active_sessions.lock();
            sessions.insert(session_id.to_string());
        }

        // 清理失效的 WebRTC 连接
        if let Some(pool) = wire_pool {
            Self::cleanup_stale_webrtc_connections(pool, session_id);
        } else {
            log::warn!("[SwLifecycle] No WirePool available for cleanup");
        }
    }

    /// 处理 DOM_UNLOADING 消息
    ///
    /// DOM 进程即将关闭时发送此消息
    fn handle_dom_unloading(active_sessions: &Arc<Mutex<HashSet<String>>>, session_id: &str) {
        if session_id.is_empty() {
            log::warn!("[SwLifecycle] DOM_UNLOADING received without session_id");
            return;
        }

        log::info!("[SwLifecycle] DOM_UNLOADING received: {}", session_id);

        // 从活跃会话集合移除
        {
            let mut sessions = active_sessions.lock();
            sessions.remove(session_id);
        }

        log::info!("[SwLifecycle] Session {} marked for cleanup", session_id);
    }

    /// 处理 DOM_PING 消息
    ///
    /// DOM 检查 SW 是否活跃
    fn handle_dom_ping(session_id: &str) {
        log::debug!("[SwLifecycle] DOM_PING received from {}", session_id);

        // TODO: 发送 PONG 响应
        // 需要有一个返回通道
    }

    /// 清理失效的 WebRTC 连接
    fn cleanup_stale_webrtc_connections(wire_pool: &Arc<WirePool>, session_id: &str) {
        log::info!(
            "[SwLifecycle] Cleaning up stale WebRTC connections for session: {}",
            session_id
        );

        // 标记 WebRTC 连接为失效
        // 注意：这里简单地移除所有 WebRTC 连接
        // 实际上可能需要更精细的会话管理
        wire_pool.mark_connection_failed(ConnType::WebRTC);

        log::info!("[SwLifecycle] WebRTC connections marked as failed");
    }

    /// 获取当前活跃会话数
    pub fn active_session_count(&self) -> usize {
        self.active_sessions.lock().len()
    }

    /// 检查会话是否活跃
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

        // 模拟添加会话
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("session-1".to_string());
            sessions.insert("session-2".to_string());
        }

        assert_eq!(manager.active_session_count(), 2);
        assert!(manager.is_session_active("session-1"));
        assert!(manager.is_session_active("session-2"));
        assert!(!manager.is_session_active("session-3"));

        // 移除会话
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

        // 验证会话被添加
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-123"));
    }

    #[test]
    fn test_handle_dom_ready_without_wire_pool() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));
        let wire_pool_opt: Option<Arc<WirePool>> = None;

        SwLifecycleManager::handle_dom_ready(&active_sessions, &wire_pool_opt, "session-456");

        // 即使没有 wire_pool，会话仍应被添加
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-456"));
    }

    #[test]
    fn test_handle_dom_ready_empty_session_id() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));
        let wire_pool = create_test_wire_pool();
        let wire_pool_opt = Some(wire_pool);

        SwLifecycleManager::handle_dom_ready(&active_sessions, &wire_pool_opt, "");

        // 空 session_id 不应被添加
        let sessions = active_sessions.lock();
        assert!(!sessions.contains(""));
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn test_handle_dom_unloading() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));

        // 先添加会话
        {
            let mut sessions = active_sessions.lock();
            sessions.insert("session-abc".to_string());
        }

        // 处理 DOM_UNLOADING
        SwLifecycleManager::handle_dom_unloading(&active_sessions, "session-abc");

        // 验证会话被移除
        let sessions = active_sessions.lock();
        assert!(!sessions.contains("session-abc"));
    }

    #[test]
    fn test_handle_dom_unloading_empty_session_id() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));

        // 添加一个会话
        {
            let mut sessions = active_sessions.lock();
            sessions.insert("session-xyz".to_string());
        }

        // 处理空 session_id
        SwLifecycleManager::handle_dom_unloading(&active_sessions, "");

        // 原有会话不应受影响
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-xyz"));
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn test_handle_dom_unloading_nonexistent_session() {
        let active_sessions = Arc::new(Mutex::new(HashSet::new()));

        // 添加一个会话
        {
            let mut sessions = active_sessions.lock();
            sessions.insert("session-1".to_string());
        }

        // 尝试移除不存在的会话
        SwLifecycleManager::handle_dom_unloading(&active_sessions, "session-999");

        // 原有会话不应受影响
        let sessions = active_sessions.lock();
        assert!(sessions.contains("session-1"));
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn test_handle_dom_ping() {
        // DOM_PING 处理应该不会崩溃
        SwLifecycleManager::handle_dom_ping("ping-session");

        // TODO: 当实现 PONG 响应后，添加更多断言
    }

    #[test]
    fn test_cleanup_stale_webrtc_connections() {
        let wire_pool = create_test_wire_pool();

        // 清理失效连接 (不会崩溃)
        SwLifecycleManager::cleanup_stale_webrtc_connections(&wire_pool, "test-session");

        // 测试通过即表示清理函数正常执行
    }

    #[test]
    fn test_multiple_sessions_management() {
        let manager = SwLifecycleManager::new();

        // 添加多个会话
        {
            let mut sessions = manager.active_sessions.lock();
            for i in 0..10 {
                sessions.insert(format!("session-{}", i));
            }
        }

        assert_eq!(manager.active_session_count(), 10);

        // 验证所有会话都活跃
        for i in 0..10 {
            assert!(manager.is_session_active(&format!("session-{}", i)));
        }

        // 移除一半会话
        {
            let mut sessions = manager.active_sessions.lock();
            for i in 0..5 {
                sessions.remove(&format!("session-{}", i));
            }
        }

        assert_eq!(manager.active_session_count(), 5);

        // 验证移除的会话不活跃
        for i in 0..5 {
            assert!(!manager.is_session_active(&format!("session-{}", i)));
        }

        // 验证保留的会话仍活跃
        for i in 5..10 {
            assert!(manager.is_session_active(&format!("session-{}", i)));
        }
    }

    #[test]
    fn test_session_reactivation() {
        let manager = SwLifecycleManager::new();

        // 添加会话
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("reactivate-session".to_string());
        }

        assert!(manager.is_session_active("reactivate-session"));

        // 移除会话
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.remove("reactivate-session");
        }

        assert!(!manager.is_session_active("reactivate-session"));

        // 重新添加相同会话
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

        // 初始没有 wire_pool
        assert!(manager.wire_pool.is_none());

        // 设置 wire_pool
        manager.set_wire_pool(wire_pool.clone());
        assert!(manager.wire_pool.is_some());

        // 验证可以访问 wire_pool
        assert!(manager.wire_pool.is_some());
    }

    #[test]
    fn test_concurrent_session_operations() {
        let manager = SwLifecycleManager::new();

        // 模拟并发添加会话
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("concurrent-1".to_string());
            sessions.insert("concurrent-2".to_string());
            sessions.insert("concurrent-3".to_string());
        }

        // 模拟并发查询
        assert!(manager.is_session_active("concurrent-1"));
        assert!(manager.is_session_active("concurrent-2"));
        assert!(manager.is_session_active("concurrent-3"));

        // 模拟并发移除
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

        // 尝试查询空 session_id
        assert!(!manager.is_session_active(""));

        // 尝试手动添加空 session_id（虽然 handle_dom_ready 会拒绝）
        {
            let mut sessions = manager.active_sessions.lock();
            sessions.insert("".to_string());
        }

        assert!(manager.is_session_active(""));
        assert_eq!(manager.active_session_count(), 1);
    }
}
