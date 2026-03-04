//! DOM Transport 实现
//!
//! 统一的传输层封装，负责：
//! - 管理 WebRTC P2P 连接池（DataChannel + MediaTrack）
//! - 管理与 SW 的 PostMessage 通道
//! - 并发尝试连接策略（P2P + WebSocket fallback）
//! - 就绪速度优先（哪个先连上用哪个）
//! - 自动路由和转发
//! - Fast Path 集成

use super::lane::DataLane;
use crate::fastpath::{MediaFrameHandlerRegistry, StreamHandlerRegistry};
use crate::keepalive::ServiceWorkerKeepalive;
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

/// 连接类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionType {
    /// P2P (WebRTC)
    P2P,
    /// WebSocket (通过 SW)
    WebSocket,
}

/// Dest 连接信息
struct DestConnection {
    /// 主连接（DataChannel 或 通过 SW 的 WebSocket）
    primary: Option<DataLane>,

    /// 连接类型
    conn_type: ConnectionType,

    /// 连接状态
    state: ConnectionState,

    /// MediaTrack Lanes（如果是 P2P）
    media_tracks: Vec<DataLane>,
}

/// DOM 端的 Transport 实现
pub struct DomTransport {
    /// 本地 ID
    local_id: String,

    /// 连接池：Dest → DestConnection
    connections: Arc<DashMap<Dest, DestConnection>>,

    /// SW 通道（PostMessage）
    sw_channel: Arc<Mutex<Option<DataLane>>>,

    /// Fast Path Registries
    stream_registry: Arc<StreamHandlerRegistry>,
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// Keepalive
    keepalive: Arc<Mutex<Option<ServiceWorkerKeepalive>>>,

    /// 连接策略
    strategy: ConnectionStrategy,

    /// 统计信息
    stats: Arc<Mutex<TransportStats>>,

    /// 接收通道
    rx: Arc<Mutex<mpsc::UnboundedReceiver<(Dest, PayloadType, Bytes)>>>,
    tx: mpsc::UnboundedSender<(Dest, PayloadType, Bytes)>,
}

impl DomTransport {
    /// 创建新的 DomTransport
    pub fn new(local_id: String, strategy: Option<ConnectionStrategy>) -> Self {
        let (tx, rx) = mpsc::unbounded();

        Self {
            local_id,
            connections: Arc::new(DashMap::new()),
            sw_channel: Arc::new(Mutex::new(None)),
            stream_registry: Arc::new(StreamHandlerRegistry::new()),
            media_registry: Arc::new(MediaFrameHandlerRegistry::new()),
            keepalive: Arc::new(Mutex::new(None)),
            strategy: strategy.unwrap_or_default(),
            stats: Arc::new(Mutex::new(TransportStats::default())),
            rx: Arc::new(Mutex::new(rx)),
            tx,
        }
    }

    /// 设置 SW 通道并启动 Keepalive
    pub fn set_sw_channel(&self, lane: DataLane) -> WebResult<()> {
        // 创建 Keepalive
        let keepalive = ServiceWorkerKeepalive::new(Arc::new(lane.clone()), None);
        keepalive.start();

        {
            let mut sw_channel = self.sw_channel.lock();
            *sw_channel = Some(lane);
        }

        {
            let mut ka = self.keepalive.lock();
            *ka = Some(keepalive);
        }

        log::info!("[DomTransport] SW channel established with keepalive");

        // 启动 SW 消息接收循环
        self.start_sw_receiver();

        Ok(())
    }

    /// 发送消息
    pub async fn send(&self, dest: &Dest, payload_type: PayloadType, data: Bytes) -> WebResult<()> {
        log::trace!(
            "[DomTransport] send: dest={:?}, payload_type={:?}, size={} bytes",
            dest,
            payload_type,
            data.len()
        );

        match payload_type {
            // RPC 转发到 SW（走 State Path）
            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                self.forward_to_sw(dest, payload_type, data).await?;
            }

            // STREAM 和 MEDIA 在 DOM 处理
            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                let data_len = data.len();

                // 尝试使用 P2P 连接
                if let Some(lane) = self.get_connection(dest).await {
                    lane.send(data).await?;
                } else {
                    // Fallback 到 SW（通过 WebSocket）
                    log::warn!("[DomTransport] P2P not available, fallback to SW for STREAM");
                    self.forward_to_sw(dest, payload_type, data).await?;
                }

                // 更新统计
                let mut stats = self.stats.lock();
                stats.bytes_sent += data_len as u64;
                stats.messages_sent += 1;
            }

            PayloadType::MediaRtp => {
                // 必须走 MediaTrack（P2P）
                if let Some(lane) = self.get_connection(dest).await {
                    let data_len = data.len();
                    lane.send(data).await?;

                    let mut stats = self.stats.lock();
                    stats.bytes_sent += data_len as u64;
                    stats.messages_sent += 1;
                } else {
                    return Err(WebError::Transport(
                        "No P2P connection available for MEDIA_RTP".to_string(),
                    ));
                }
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

    /// 连接到目标（并发尝试策略）
    ///
    /// **核心策略**：
    /// 1. 并发尝试 P2P 和 WebSocket（通过 SW）
    /// 2. 就绪速度优先：哪个先成功用哪个
    /// 3. P2P 优先级更高，如果都成功优先使用 P2P
    pub async fn connect(&self, dest: &Dest) -> WebResult<()> {
        // 检查是否已连接
        if let Some(entry) = self.connections.get(dest) {
            if entry.state == ConnectionState::Connected {
                log::debug!("[DomTransport] Already connected to {:?}", dest);
                return Ok(());
            }
        }

        // 标记为连接中
        self.connections.insert(
            dest.clone(),
            DestConnection {
                primary: None,
                conn_type: ConnectionType::P2P,
                state: ConnectionState::Connecting,
                media_tracks: Vec::new(),
            },
        );

        // 并发尝试连接
        let result = if self.strategy.concurrent_attempts {
            self.concurrent_connect(dest).await
        } else {
            self.sequential_connect(dest).await
        };

        match result {
            Ok(conn) => {
                // 更新为已连接状态
                self.connections.insert(dest.clone(), conn);

                log::info!("[DomTransport] Connected to {:?}", dest);

                Ok(())
            }
            Err(e) => {
                // 标记为失败
                self.connections.remove(dest);

                Err(e)
            }
        }
    }

    /// 并发连接策略（就绪速度优先）
    ///
    /// 同时尝试：
    /// 1. P2P (WebRTC DataChannel)
    /// 2. WebSocket (通过 SW)
    ///
    /// **就绪速度优先**：哪个先连上用哪个
    /// **优先级调整**：如果都成功，优先使用 P2P
    async fn concurrent_connect(&self, dest: &Dest) -> WebResult<DestConnection> {
        use futures::future::FutureExt;

        log::debug!("[DomTransport] Concurrent connect to {:?}", dest);

        // 创建两个连接任务
        let p2p_future = self.create_p2p_connection(dest).fuse();
        let websocket_future = self.create_websocket_fallback(dest).fuse();

        futures::pin_mut!(p2p_future, websocket_future);

        // 使用 select! 并发尝试
        let mut p2p_result = None;
        let mut ws_result = None;

        // 第一轮：等待第一个成功
        futures::select! {
            result = p2p_future => {
                p2p_result = Some(result);
            }
            result = websocket_future => {
                ws_result = Some(result);
            }
        }

        // 如果第一个成功是 P2P，直接使用
        if let Some(Ok(conn)) = p2p_result {
            log::info!("[DomTransport] P2P connected first (concurrent mode)");
            return Ok(conn);
        }

        // 如果第一个成功是 WebSocket，检查 P2P 是否也成功了
        if let Some(Ok(ws_conn)) = ws_result {
            // 继续等待 P2P（短暂等待，如果P2P优先级更高）
            if self.strategy.p2p_priority > self.strategy.websocket_priority {
                // 等待最多100ms，看P2P能否成功
                let timeout = async {
                    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(
                        &mut |resolve, _| {
                            let window = web_sys::window().unwrap();
                            window
                                .set_timeout_with_callback_and_timeout_and_arguments_0(
                                    &resolve, 100,
                                )
                                .unwrap();
                        },
                    ))
                    .await
                    .ok();
                };

                futures::select! {
                    result = p2p_future => {
                        if let Ok(p2p_conn) = result {
                            log::info!("[DomTransport] P2P also succeeded, using P2P (priority)");
                            return Ok(p2p_conn);
                        }
                    }
                    _ = timeout.fuse() => {
                        log::info!("[DomTransport] WebSocket connected first, P2P timeout");
                    }
                }
            }

            log::info!("[DomTransport] WebSocket connected (concurrent mode)");
            return Ok(ws_conn);
        }

        // 都失败了
        Err(WebError::Transport(format!(
            "Failed to connect to {:?}: both P2P and WebSocket failed",
            dest
        )))
    }

    /// 顺序连接策略（P2P 优先，失败则 WebSocket）
    async fn sequential_connect(&self, dest: &Dest) -> WebResult<DestConnection> {
        log::debug!("[DomTransport] Sequential connect to {:?}", dest);

        // 先尝试 P2P
        match self.create_p2p_connection(dest).await {
            Ok(conn) => {
                log::info!("[DomTransport] P2P connected (sequential mode)");
                Ok(conn)
            }
            Err(e) => {
                log::warn!("[DomTransport] P2P failed: {:?}, trying WebSocket", e);

                // Fallback 到 WebSocket
                self.create_websocket_fallback(dest).await
            }
        }
    }

    /// 创建 P2P 连接（WebRTC DataChannel + MediaTrack）
    async fn create_p2p_connection(&self, _dest: &Dest) -> WebResult<DestConnection> {
        // TODO: 实现 WebRTC 信令和 PeerConnection 创建
        // 这里需要：
        // 1. 信令交换（SDP offer/answer）
        // 2. ICE candidate 交换
        // 3. 创建 DataChannel
        // 4. 创建 MediaTrack（如果需要）

        // 暂时返回错误（Phase 4 实现）
        Err(WebError::Transport(
            "P2P connection not implemented yet (Phase 4)".to_string(),
        ))
    }

    /// 创建 WebSocket Fallback（通过 SW）
    async fn create_websocket_fallback(&self, dest: &Dest) -> WebResult<DestConnection> {
        log::debug!("[DomTransport] Creating WebSocket fallback for {:?}", dest);

        // 通过 SW 建立 WebSocket 连接
        // 这里不需要实际的 Lane，因为消息都会通过 SW 转发
        // 只需要标记连接类型为 WebSocket

        Ok(DestConnection {
            primary: None, // 通过 SW 转发，不需要本地 Lane
            conn_type: ConnectionType::WebSocket,
            state: ConnectionState::Connected,
            media_tracks: Vec::new(),
        })
    }

    /// 获取连接
    async fn get_connection(&self, dest: &Dest) -> Option<DataLane> {
        if let Some(entry) = self.connections.get(dest) {
            if entry.state == ConnectionState::Connected {
                return entry.primary.clone();
            }
        }

        None
    }

    /// 转发到 SW
    async fn forward_to_sw(
        &self,
        dest: &Dest,
        payload_type: PayloadType,
        data: Bytes,
    ) -> WebResult<()> {
        let sw_channel = self.sw_channel.lock();

        if let Some(lane) = sw_channel.as_ref() {
            let forward_msg = ForwardMessage::new(dest.clone(), payload_type, data);
            let serialized = forward_msg.serialize()?;

            lane.send(serialized).await?;

            log::trace!(
                "[DomTransport] Forwarded to SW: dest={:?}, payload_type={:?}",
                dest,
                payload_type
            );

            Ok(())
        } else {
            Err(WebError::Transport("SW channel not available".to_string()))
        }
    }

    /// 启动 SW 消息接收循环
    fn start_sw_receiver(&self) {
        let sw_channel = self.sw_channel.clone();
        let tx = self.tx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let lane = {
                    let channel = sw_channel.lock();
                    channel.as_ref().cloned()
                };

                if let Some(lane) = lane {
                    match lane.recv().await {
                        Some(data) => match ForwardMessage::deserialize(&data) {
                            Ok(forward_msg) => {
                                log::trace!(
                                    "[DomTransport] Received from SW: dest={:?}, payload_type={:?}",
                                    forward_msg.dest,
                                    forward_msg.payload_type
                                );

                                if let Err(e) = tx.unbounded_send((
                                    forward_msg.dest,
                                    forward_msg.payload_type,
                                    forward_msg.data,
                                )) {
                                    log::error!(
                                        "[DomTransport] Failed to forward SW message: {:?}",
                                        e
                                    );
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "[DomTransport] Failed to parse forward message: {:?}",
                                    e
                                );
                            }
                        },
                        None => {
                            log::warn!("[DomTransport] SW receiver closed");
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
        self.connections.remove(dest);
        log::info!("[DomTransport] Disconnected: {:?}", dest);
        Ok(())
    }

    /// 获取连接状态
    pub fn connection_state(&self, dest: &Dest) -> ConnectionState {
        self.connections
            .get(dest)
            .map(|entry| entry.state)
            .unwrap_or(ConnectionState::Disconnected)
    }

    /// 获取统计信息
    pub fn stats(&self) -> TransportStats {
        self.stats.lock().clone()
    }

    /// 获取 Stream Registry
    pub fn stream_registry(&self) -> &Arc<StreamHandlerRegistry> {
        &self.stream_registry
    }

    /// 获取 Media Registry
    pub fn media_registry(&self) -> &Arc<MediaFrameHandlerRegistry> {
        &self.media_registry
    }
}
