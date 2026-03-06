//! OutprocTransportManager - 跨进程传输管理器
//!
//! 管理多个 Dest 的传输层，提供统一的 send/recv 接口

use super::dest_transport::DestTransport;
use super::wire_builder::WireBuilder;
use actr_web_common::{Dest, PayloadType, WebResult};
use dashmap::DashMap;
use std::sync::Arc;

/// Dest 传输状态
///
/// 使用 Either 模式管理连接生命周期：
/// - Left: Connecting 状态（多个等待者）
/// - Right: Connected 状态（DestTransport）
enum DestState {
    /// 正在连接（等待创建完成）
    Connecting(Arc<futures::channel::oneshot::Receiver<Arc<DestTransport>>>),

    /// 已连接
    Connected(Arc<DestTransport>),
}

/// OutprocTransportManager - 跨进程传输管理器
///
/// 职责：
/// - 管理多个 Dest 的传输层（每个 Dest 映射到一个 DestTransport）
/// - 按需创建 DestTransport（lazy 初始化）
/// - 提供统一的 send/recv 接口
/// - 支持自定义连接工厂
/// - 防止重复创建连接（使用 DashMap + oneshot）
pub struct OutprocTransportManager {
    /// 本地 ID
    local_id: String,

    /// Dest → DestState 映射
    transports: Arc<DashMap<Dest, DestState>>,

    /// Wire 构建器
    wire_builder: Arc<dyn WireBuilder>,
}

impl OutprocTransportManager {
    /// 创建新的 OutprocTransportManager
    ///
    /// # 参数
    /// - `local_id`: 本地 Actor ID 或标识符
    /// - `wire_builder`: Wire 构建器，异步创建基于 Dest 的 Wire 句柄列表
    pub fn new(local_id: String, wire_builder: Arc<dyn WireBuilder>) -> Self {
        Self {
            local_id,
            transports: Arc::new(DashMap::new()),
            wire_builder,
        }
    }

    /// 获取或创建指定 Dest 的 DestTransport
    ///
    /// # 参数
    /// - `dest`: 目标地址
    ///
    /// # 返回
    /// - 该 Dest 的 DestTransport（Arc 共享）
    ///
    /// # 状态机
    /// 使用 DashMap + oneshot 防止重复连接：
    /// 1. 如果 Connected → 返回 transport
    /// 2. 如果 Connecting → 等待 oneshot，然后返回
    /// 3. 如果 None → 插入 Connecting，创建连接，完成后更新为 Connected
    pub async fn get_or_create_transport(&self, dest: &Dest) -> WebResult<Arc<DestTransport>> {
        // 1. 快速路径：检查是否已存在
        if let Some(entry) = self.transports.get(dest) {
            match entry.value() {
                DestState::Connected(transport) => {
                    log::debug!(
                        "[OutprocTransportManager] Reusing existing DestTransport: {:?}",
                        dest
                    );
                    return Ok(Arc::clone(transport));
                }
                DestState::Connecting(_rx) => {
                    // 等待创建完成
                    log::debug!(
                        "[OutprocTransportManager] Waiting for ongoing connection: {:?}",
                        dest
                    );
                    drop(entry); // 释放锁

                    // 注意：oneshot receiver 只能接收一次，这里需要特殊处理
                    // 简化实现：重新检查
                    loop {
                        if let Some(entry) = self.transports.get(dest) {
                            if let DestState::Connected(transport) = entry.value() {
                                return Ok(Arc::clone(transport));
                            }
                        }

                        // 简单延迟重试（使用 gloo_timers，兼容 Service Worker 环境）
                        gloo_timers::future::TimeoutFuture::new(10).await;
                    }
                }
            }
        }

        // 2. 慢速路径：创建新连接
        log::info!(
            "[OutprocTransportManager] Creating new connection for: {:?}",
            dest
        );

        // 创建 oneshot channel
        let (tx, rx) = futures::channel::oneshot::channel();

        // 尝试插入 Connecting 状态
        let inserted = self
            .transports
            .insert(dest.clone(), DestState::Connecting(Arc::new(rx)))
            .is_none();

        if !inserted {
            // 其他线程已经插入了，等待它完成
            return Box::pin(self.get_or_create_transport(dest)).await;
        }

        // 我们是创建者，开始创建连接
        let result = async {
            let connections = self.wire_builder.create_connections(dest).await?;

            // 允许 0 连接启动：WireBuilder 可能只是触发了异步连接创建（如请求 DOM 建立 P2P），
            // 实际的 WireHandle 稍后通过 inject_connection 注入。
            // DestTransport 的事件驱动发送循环会等待 ReadyWatcher。
            log::info!(
                "[OutprocTransportManager] Creating DestTransport: {:?} ({} initial connections)",
                dest,
                connections.len()
            );

            let transport = DestTransport::new(dest.clone(), connections).await?;
            Ok(Arc::new(transport))
        }
        .await;

        // 更新状态
        match result {
            Ok(transport) => {
                log::info!(
                    "[OutprocTransportManager] Connection established: {:?}",
                    dest
                );
                self.transports
                    .insert(dest.clone(), DestState::Connected(Arc::clone(&transport)));

                // 通知等待者
                tx.send(Arc::clone(&transport)).ok();

                Ok(transport)
            }
            Err(e) => {
                log::error!(
                    "[OutprocTransportManager] Connection failed: {:?}: {}",
                    dest,
                    e
                );
                self.transports.remove(dest);

                // 通知等待者失败（通过关闭 channel）
                drop(tx);

                Err(e)
            }
        }
    }

    /// 发送消息到指定 Dest
    ///
    /// # 参数
    /// - `dest`: 目标地址
    /// - `payload_type`: 消息类型
    /// - `data`: 消息数据
    pub async fn send(&self, dest: &Dest, payload_type: PayloadType, data: &[u8]) -> WebResult<()> {
        log::debug!(
            "[OutprocTransportManager] Sending to {:?}: type={:?}, size={}",
            dest,
            payload_type,
            data.len()
        );

        // 获取或创建 DestTransport
        let transport = self.get_or_create_transport(dest).await?;

        // 通过 DestTransport 发送
        transport.send(payload_type, data).await
    }

    /// 关闭指定 Dest 的 DestTransport
    ///
    /// # 参数
    /// - `dest`: 目标地址
    pub async fn close_transport(&self, dest: &Dest) -> WebResult<()> {
        if let Some((_, state)) = self.transports.remove(dest) {
            match state {
                DestState::Connected(transport) => {
                    log::info!(
                        "[OutprocTransportManager] Closing DestTransport: {:?}",
                        dest
                    );
                    transport.close().await?;
                }
                DestState::Connecting(_) => {
                    log::debug!(
                        "[OutprocTransportManager] Removed Connecting state for: {:?}",
                        dest
                    );
                }
            }
        }

        Ok(())
    }

    /// 关闭所有 DestTransports
    pub async fn close_all(&self) -> WebResult<()> {
        log::info!(
            "[OutprocTransportManager] Closing all DestTransports (count: {})",
            self.transports.len()
        );

        let dests: Vec<Dest> = self
            .transports
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        for dest in dests {
            if let Err(e) = self.close_transport(&dest).await {
                log::warn!(
                    "[OutprocTransportManager] Failed to close DestTransport {:?}: {}",
                    dest,
                    e
                );
            }
        }

        Ok(())
    }

    /// 获取当前管理的 Dest 数量
    pub fn dest_count(&self) -> usize {
        self.transports.len()
    }

    /// 获取本地 ID
    #[inline]
    pub fn local_id(&self) -> &str {
        &self.local_id
    }

    /// 列出所有已连接的 Dests
    pub fn list_dests(&self) -> Vec<Dest> {
        self.transports
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// 检查到指定 Dest 的连接是否存在
    pub fn has_dest(&self, dest: &Dest) -> bool {
        self.transports.contains_key(dest)
    }

    /// 向指定 Dest 的 WirePool 注入新连接
    ///
    /// 用于 DOM 建立 P2P 后传入专用 MessagePort 的场景：
    /// 1. datachannel_open 事件触发
    /// 2. SW 收到 DOM 转移的 MessagePort
    /// 3. 创建 WebRtcConnection → WireHandle::WebRTC
    /// 4. 调用此方法注入到对应 Dest 的 WirePool
    /// 5. WirePool ReadyWatcher 通知 → DestTransport send 循环唤醒
    ///
    /// 如果对应 Dest 的 DestTransport 尚未创建，会自动创建一个空的。
    pub async fn inject_connection(
        &self,
        dest: &Dest,
        wire_handle: super::wire_handle::WireHandle,
    ) -> WebResult<()> {
        let transport = self.get_or_create_transport(dest).await?;
        transport.wire_pool().add_connection(wire_handle);
        log::info!(
            "[OutprocTransportManager] Injected connection into {:?}",
            dest
        );
        Ok(())
    }
}

impl Drop for OutprocTransportManager {
    fn drop(&mut self) {
        log::debug!("[OutprocTransportManager] Dropped");
        // 注意：异步清理需要外部调用 close_all()
    }
}
