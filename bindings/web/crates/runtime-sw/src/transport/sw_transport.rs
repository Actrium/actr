//! Service Worker Transport 实现
//!
//! 统一的传输层封装，负责：
//! - 管理 WebSocket 连接池
//! - 管理与 DOM 的 PostMessage 通道
//! - 自动路由消息（根据 PayloadType）
//! - 自动转发（MEDIA_RTP → DOM）

use super::lane::DataLane;
use actr_web_common::{
    ConnectionState, ConnectionStrategy, Dest, ForwardMessage, PayloadType, TransportStats,
    WebError, WebResult,
};
use bytes::Bytes;
use dashmap::DashMap;
use futures::StreamExt;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
// 移除 tokio 依赖，Web 环境不支持

use super::websocket::WebSocketLaneBuilder;

/// Service Worker 端的 Transport 实现
pub struct SwTransport {
    /// 本地 ID
    local_id: String,

    /// WebSocket 连接池：Dest → (DataLane, ConnectionState)
    websocket_pool: Arc<DashMap<Dest, (DataLane, ConnectionState)>>,

    /// DOM 通道（PostMessage）
    dom_channel: Arc<Mutex<Option<DataLane>>>,

    /// 连接策略
    strategy: ConnectionStrategy,

    /// 统计信息
    stats: Arc<Mutex<TransportStats>>,

    /// 接收通道（汇总所有消息）
    rx: Arc<Mutex<mpsc::UnboundedReceiver<(Dest, PayloadType, Bytes)>>>,
    tx: mpsc::UnboundedSender<(Dest, PayloadType, Bytes)>,
}

impl SwTransport {
    /// 创建新的 SwTransport
    pub fn new(local_id: String, strategy: Option<ConnectionStrategy>) -> Self {
        let (tx, rx) = mpsc::unbounded();

        Self {
            local_id,
            websocket_pool: Arc::new(DashMap::new()),
            dom_channel: Arc::new(Mutex::new(None)),
            strategy: strategy.unwrap_or_default(),
            stats: Arc::new(Mutex::new(TransportStats::default())),
            rx: Arc::new(Mutex::new(rx)),
            tx,
        }
    }

    /// 设置 DOM 通道
    ///
    /// 当 DOM 通过 MessagePort 连接到 SW 时调用
    pub fn set_dom_channel(&self, lane: DataLane) -> WebResult<()> {
        let mut dom_channel = self.dom_channel.lock();
        *dom_channel = Some(lane);

        log::info!("[SwTransport] DOM channel established");

        // 启动 DOM 消息接收循环
        self.start_dom_receiver();

        Ok(())
    }

    /// 发送消息
    pub async fn send(&self, dest: &Dest, payload_type: PayloadType, data: Bytes) -> WebResult<()> {
        log::trace!(
            "[SwTransport] send: dest={:?}, payload_type={:?}, size={} bytes",
            dest,
            payload_type,
            data.len()
        );

        // 路由策略
        match payload_type {
            // RPC 和 STREAM 在 SW 处理（通过 WebSocket）
            PayloadType::RpcReliable
            | PayloadType::RpcSignal
            | PayloadType::StreamReliable
            | PayloadType::StreamLatencyFirst => {
                let lane = self.get_or_create_websocket(dest).await?;
                lane.send(data.clone()).await?;

                // 更新统计
                let mut stats = self.stats.lock();
                stats.bytes_sent += data.len() as u64;
                stats.messages_sent += 1;
            }

            // MEDIA_RTP 必须转发到 DOM
            PayloadType::MediaRtp => {
                self.forward_to_dom(dest, payload_type, data).await?;
            }
        }

        Ok(())
    }

    /// 接收消息
    pub async fn recv(&self) -> Option<(Dest, PayloadType, Bytes)> {
        let mut rx = self.rx.lock();
        let msg = rx.next().await;

        if let Some((_, _, ref data)) = msg {
            let mut stats = self.stats.lock();
            stats.bytes_received += data.len() as u64;
            stats.messages_received += 1;
        }

        msg
    }

    /// 获取统计信息
    pub fn stats(&self) -> TransportStats {
        self.stats.lock().clone()
    }

    /// 获取或创建 WebSocket 连接
    ///
    /// 简化实现：直接创建，由 DashMap 处理并发
    async fn get_or_create_websocket(&self, dest: &Dest) -> WebResult<DataLane> {
        // 1. 快速路径：检查是否已存在
        if let Some(entry) = self.websocket_pool.get(dest) {
            let (lane, state) = entry.value();
            if *state == ConnectionState::Connected {
                return Ok(lane.clone());
            }
        }

        // 2. 创建新连接
        let lane = self.create_websocket_connection(dest).await?;

        // 更新为已连接状态
        self.websocket_pool
            .insert(dest.clone(), (lane.clone(), ConnectionState::Connected));

        log::info!("[SwTransport] WebSocket connected: {:?}", dest);

        // 启动接收循环
        self.start_websocket_receiver(dest.clone(), lane.clone());

        Ok(lane)
    }

    /// 创建 WebSocket 连接
    async fn create_websocket_connection(&self, dest: &Dest) -> WebResult<DataLane> {
        let url = dest.to_websocket_url()?;

        log::debug!("[SwTransport] Creating WebSocket connection to: {}", url);

        let lane = WebSocketLaneBuilder::new(url, PayloadType::RpcReliable)
            .build()
            .await?;

        Ok(lane)
    }

    /// 转发到 DOM
    async fn forward_to_dom(
        &self,
        dest: &Dest,
        payload_type: PayloadType,
        data: Bytes,
    ) -> WebResult<()> {
        let dom_channel = self.dom_channel.lock();

        if let Some(lane) = dom_channel.as_ref() {
            let forward_msg = ForwardMessage::new(dest.clone(), payload_type, data);
            let serialized = forward_msg.serialize()?;

            lane.send(serialized).await?;

            log::trace!(
                "[SwTransport] Forwarded to DOM: dest={:?}, payload_type={:?}",
                dest,
                payload_type
            );

            Ok(())
        } else {
            Err(WebError::Transport(
                "DOM channel not available, cannot forward MEDIA_RTP".to_string(),
            ))
        }
    }

    /// 启动 WebSocket 接收循环
    fn start_websocket_receiver(&self, dest: Dest, lane: DataLane) {
        let tx = self.tx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                match lane.recv().await {
                    Some(data) => {
                        // 解析 PayloadType（从消息头提取）
                        // 这里简化处理，实际应该从 Lane 获取
                        let payload_type = lane.payload_type();

                        if let Err(e) = tx.unbounded_send((dest.clone(), payload_type, data)) {
                            log::error!(
                                "[SwTransport] Failed to forward received message: {:?}",
                                e
                            );
                            break;
                        }
                    }
                    None => {
                        log::warn!("[SwTransport] WebSocket receiver closed: {:?}", dest);
                        break;
                    }
                }
            }
        });
    }

    /// 启动 DOM 消息接收循环
    fn start_dom_receiver(&self) {
        let dom_channel = self.dom_channel.clone();
        let tx = self.tx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let lane = {
                    let channel = dom_channel.lock();
                    channel.as_ref().cloned()
                };

                if let Some(lane) = lane {
                    match lane.recv().await {
                        Some(data) => {
                            // 解析转发消息
                            match ForwardMessage::deserialize(&data) {
                                Ok(forward_msg) => {
                                    log::trace!(
                                        "[SwTransport] Received from DOM: dest={:?}, payload_type={:?}",
                                        forward_msg.dest,
                                        forward_msg.payload_type
                                    );

                                    if let Err(e) = tx.unbounded_send((
                                        forward_msg.dest,
                                        forward_msg.payload_type,
                                        forward_msg.data,
                                    )) {
                                        log::error!(
                                            "[SwTransport] Failed to forward DOM message: {:?}",
                                            e
                                        );
                                    }
                                }
                                Err(e) => {
                                    log::error!(
                                        "[SwTransport] Failed to parse forward message: {:?}",
                                        e
                                    );
                                }
                            }
                        }
                        None => {
                            log::warn!("[SwTransport] DOM receiver closed");
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
        });
    }

    /// 断开连接
    pub async fn disconnect(&self, dest: &Dest) -> WebResult<()> {
        self.websocket_pool.remove(dest);
        log::info!("[SwTransport] Disconnected: {:?}", dest);
        Ok(())
    }

    /// 获取连接状态
    pub fn connection_state(&self, dest: &Dest) -> ConnectionState {
        self.websocket_pool
            .get(dest)
            .map(|entry| entry.value().1)
            .unwrap_or(ConnectionState::Disconnected)
    }
}
