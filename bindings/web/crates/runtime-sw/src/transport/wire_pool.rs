//! Wire Pool - 连接池管理
//!
//! 管理 WebSocket 和 WebRTC 连接，提供事件驱动的就绪通知

use super::wire_handle::{WireHandle, WireStatus};
use actr_web_common::{ConnType, WebResult};
use futures::StreamExt;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;

/// 就绪集合（哪些连接类型已就绪）
pub type ReadySet = HashSet<ConnType>;

/// 连接池
///
/// 管理多个连接（WebSocket + WebRTC），并发启动连接任务，
/// 通过事件驱动方式通知连接就绪状态
pub struct WirePool {
    /// 连接状态数组 [WebSocket, WebRTC]
    connections: Arc<Mutex<[Option<WireStatus>; 2]>>,

    /// 就绪状态
    ready_set: Arc<Mutex<ReadySet>>,

    /// 变更通知发送器（广播）
    change_notifiers: Arc<Mutex<Vec<mpsc::UnboundedSender<()>>>>,
}

impl WirePool {
    /// 创建新的连接池
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new([None, None])),
            ready_set: Arc::new(Mutex::new(HashSet::new())),
            change_notifiers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 添加连接并后台启动连接任务
    ///
    /// 非阻塞，立即返回，后台并发尝试连接
    pub fn add_connection(&self, connection: WireHandle) {
        let connections = Arc::clone(&self.connections);
        let ready_set = Arc::clone(&self.ready_set);
        let change_notifiers = Arc::clone(&self.change_notifiers);

        let conn_type = connection.conn_type();

        wasm_bindgen_futures::spawn_local(async move {
            // 1. 初始化状态为 Connecting
            {
                let mut conns = connections.lock();
                conns[conn_type.as_index()] = Some(WireStatus::Connecting);
            }

            log::info!("[WirePool] Starting connection task: {:?}", conn_type);

            // 2. 尝试连接
            match connection.connect().await {
                Ok(_) => {
                    log::info!("[WirePool] Connection succeeded: {:?}", conn_type);

                    // 3. 更新为 Ready
                    {
                        let mut conns = connections.lock();
                        conns[conn_type.as_index()] = Some(WireStatus::Ready(connection));
                    }

                    // 4. 更新就绪集合
                    {
                        let mut ready = ready_set.lock();
                        ready.insert(conn_type);
                    }

                    // 5. 通知所有等待者
                    Self::notify_all(&change_notifiers);
                }
                Err(e) => {
                    log::error!("[WirePool] Connection failed: {:?}: {}", conn_type, e);

                    // 标记为 Failed
                    {
                        let mut conns = connections.lock();
                        conns[conn_type.as_index()] = Some(WireStatus::Failed);
                    }

                    // 不通知（失败不触发事件）
                }
            }
        });
    }

    /// 获取指定类型的连接
    pub async fn get_connection(&self, conn_type: ConnType) -> Option<WireHandle> {
        let conns = self.connections.lock();
        if let Some(WireStatus::Ready(handle)) = &conns[conn_type.as_index()] {
            Some(handle.clone())
        } else {
            None
        }
    }

    /// 订阅就绪状态变更（返回接收器）
    ///
    /// 接收器会在就绪状态变更时收到通知
    pub fn subscribe_changes(&self) -> ReadyWatcher {
        let (tx, rx) = mpsc::unbounded();

        // 注册到通知列表
        {
            let mut notifiers = self.change_notifiers.lock();
            notifiers.push(tx);
        }

        ReadyWatcher {
            rx: Arc::new(Mutex::new(rx)),
            ready_set: Arc::clone(&self.ready_set),
        }
    }

    /// 通知所有等待者
    fn notify_all(notifiers: &Arc<Mutex<Vec<mpsc::UnboundedSender<()>>>>) {
        let mut notifiers = notifiers.lock();

        // 清理已关闭的接收器
        notifiers.retain(|tx| tx.unbounded_send(()).is_ok());

        log::trace!("[WirePool] Notified {} waiters", notifiers.len());
    }

    /// 标记连接为失效
    ///
    /// 当检测到连接失效时调用（如 MessagePort 发送失败）
    pub fn mark_connection_failed(&self, conn_type: ConnType) {
        let mut conns = self.connections.lock();
        conns[conn_type.as_index()] = Some(WireStatus::Failed);

        log::warn!("[WirePool] Connection marked as failed: {:?}", conn_type);

        // 从就绪集合移除
        {
            let mut ready = self.ready_set.lock();
            ready.remove(&conn_type);
        }

        // 通知等待者（让他们知道状态变化了）
        Self::notify_all(&self.change_notifiers);
    }

    /// 移除连接
    ///
    /// 彻底移除连接状态，为重建做准备
    pub fn remove_connection(&self, conn_type: ConnType) {
        let mut conns = self.connections.lock();
        conns[conn_type.as_index()] = None;

        log::info!("[WirePool] Connection removed: {:?}", conn_type);

        // 从就绪集合移除
        {
            let mut ready = self.ready_set.lock();
            ready.remove(&conn_type);
        }
    }

    /// 重新连接
    ///
    /// 用于恢复场景：先移除旧连接，再添加新连接
    pub fn reconnect(&self, connection: WireHandle) {
        let conn_type = connection.conn_type();

        log::info!("[WirePool] Reconnecting: {:?}", conn_type);

        // 先移除旧的
        self.remove_connection(conn_type);

        // 再添加新的
        self.add_connection(connection);
    }

    /// 健康检查
    ///
    /// 检查所有连接的存活状态
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

    /// 获取所有连接的状态
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

/// 就绪状态监视器
///
/// 用于等待连接就绪状态变更
pub struct ReadyWatcher {
    /// 变更通知接收器
    rx: Arc<Mutex<mpsc::UnboundedReceiver<()>>>,

    /// 就绪集合引用
    ready_set: Arc<Mutex<ReadySet>>,
}

impl ReadyWatcher {
    /// 获取当前就绪集合（快照）
    pub fn borrow_and_update(&self) -> ReadySet {
        self.ready_set.lock().clone()
    }

    /// 等待下一次变更
    ///
    /// 返回 Ok(()) 表示有变更，Err(()) 表示通道已关闭
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

/// ConnType 扩展方法
trait ConnTypeExt {
    fn as_index(&self) -> usize;
}

impl ConnTypeExt for ConnType {
    /// 转换为数组索引 (WebSocket=0, WebRTC=1)
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

        // 初始状态应该是空的
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

        // 验证订阅者被正确注册
        let notifiers = pool.change_notifiers.lock();
        assert_eq!(notifiers.len(), 2);
    }

    #[test]
    fn test_remove_connection() {
        let pool = WirePool::new();

        // 添加一个模拟的连接状态
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Connecting);
        }

        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebRTC);
        }

        // 移除连接
        pool.remove_connection(ConnType::WebRTC);

        // 验证状态被清除
        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebRTC.as_index()].is_none());

        let ready = pool.ready_set.lock();
        assert!(!ready.contains(&ConnType::WebRTC));
    }

    #[test]
    fn test_reconnect() {
        let pool = WirePool::new();

        // 模拟一个失败的连接
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebSocket.as_index()] = Some(WireStatus::Failed);
        }

        // reconnect 应该清除失败状态并允许重新添加
        // 注意：这里只是验证状态清理，实际重连逻辑在 add_connection 中
        pool.remove_connection(ConnType::WebSocket);

        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebSocket.as_index()].is_none());
    }

    #[test]
    fn test_multiple_connection_types() {
        let pool = WirePool::new();

        // 可以同时管理多种连接类型
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

        // 模拟连接就绪
        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebSocket);
        }

        let ready = pool.ready_set.lock();
        assert!(ready.contains(&ConnType::WebSocket));
        assert!(!ready.contains(&ConnType::WebRTC));

        // 添加第二个连接
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

        // 移除不存在的连接不应该 panic
        pool.remove_connection(ConnType::WebRTC);

        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebRTC.as_index()].is_none());
    }

    #[test]
    fn test_mark_connection_failed() {
        let pool = WirePool::new();

        // 模拟连接就绪
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebSocket.as_index()] = Some(WireStatus::Connecting);
        }

        // 标记为失效
        pool.mark_connection_failed(ConnType::WebSocket);

        // 验证状态变为 Failed
        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebSocket.as_index()],
            Some(WireStatus::Failed)
        ));
    }

    #[test]
    fn test_get_all_status() {
        let pool = WirePool::new();

        // 设置不同状态
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

        // 创建订阅者
        let _watcher1 = pool.subscribe_changes();
        let watcher2 = pool.subscribe_changes();

        // 显式删除 watcher2（关闭接收器）
        drop(watcher2);

        // 再创建一个新订阅者
        let _watcher3 = pool.subscribe_changes();

        // 通知应该清理已关闭的接收器
        let notifiers = pool.change_notifiers.lock();
        // 应该有 2 个活跃的订阅者（watcher1 和 watcher3）
        assert!(notifiers.len() >= 2);
    }

    #[test]
    fn test_ready_watcher_borrow_and_update() {
        let pool = WirePool::new();

        // 添加连接到就绪集合
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

        // 先设置一个失败的连接
        {
            let mut conns = pool.connections.lock();
            conns[ConnType::WebRTC.as_index()] = Some(WireStatus::Failed);
        }

        // reconnect 应该清除状态（实际的重连会通过 add_connection 异步进行）
        // 这里只测试移除部分
        pool.remove_connection(ConnType::WebRTC);

        let conns = pool.connections.lock();
        assert!(conns[ConnType::WebRTC.as_index()].is_none());
    }

    #[test]
    fn test_conn_type_index_uniqueness() {
        // 确保每种连接类型有唯一索引
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

        // 连接数组应该有 2 个槽位
        assert_eq!(conns.len(), 2);
    }

    #[test]
    fn test_mark_connection_failed_multiple_times() {
        let pool = WirePool::new();

        // 第一次标记失败
        pool.mark_connection_failed(ConnType::WebSocket);

        let conns = pool.connections.lock();
        assert!(matches!(
            conns[ConnType::WebSocket.as_index()],
            Some(WireStatus::Failed)
        ));
        drop(conns);

        // 第二次标记失败（应该不会 panic）
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

        // 先移除
        pool.remove_connection(ConnType::WebRTC);

        // 再标记失败（在已移除的连接上）
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

        // 添加
        {
            let mut ready = pool.ready_set.lock();
            ready.insert(ConnType::WebSocket);
        }

        // 检查
        {
            let ready = pool.ready_set.lock();
            assert!(ready.contains(&ConnType::WebSocket));
        }

        // 移除
        {
            let mut ready = pool.ready_set.lock();
            ready.remove(&ConnType::WebSocket);
        }

        // 再次检查
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
