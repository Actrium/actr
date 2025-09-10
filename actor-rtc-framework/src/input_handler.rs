//! Input Handler - 流量分诊与压力传导
//!
//! Input Handler 是 ActorSystem 的"前门"，所有来自外部世界的网络流量都必须先经过它的处理。
//! 它实现了高效的流量分诊(Triage)和灵敏的压力传导(Pressure Propagation)。

use crate::context::Context;
use crate::error::{ActorError, ActorResult};
use crate::messaging::{InternalMessage, MessagePriority};
use crate::routing::MessageScheduler;
use shared_protocols::actor::ActorId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
use webrtc::data_channel::RTCDataChannel;

/// Input Handler - 所有外部流量的智能路由器
pub struct InputHandler {
    /// 状态路径发送端 (有界 channel - 关键的背压点)
    state_path_tx: mpsc::Sender<StatePathMessage>,
    /// 快车道注册表引用
    scheduler: Arc<MessageScheduler>,
    /// 数据通道映射 (channel_label -> triage_strategy)
    channel_strategies: Arc<RwLock<HashMap<String, TriageStrategy>>>,
    /// 统计信息
    stats: Arc<tokio::sync::Mutex<HandlerStats>>,
}

/// 分诊策略
#[derive(Debug, Clone)]
pub enum TriageStrategy {
    /// 基于 DataChannel 标签的物理隔离
    ChannelBased {
        /// 控制通道标签 (所有状态类消息通过此通道)
        control_channel: String,
        /// 流数据通道前缀 (stream_id 作为通道标签)
        stream_channel_prefix: String,
    },
    /// 基于消息信封的逻辑隔离
    EnvelopeBased {
        /// 单一 DataChannel 处理所有流量
        unified_channel: String,
    },
}

/// 状态路径消息包装
#[derive(Debug)]
pub struct StatePathMessage {
    #[allow(dead_code)]
    pub(crate) message: InternalMessage,
    pub context: Arc<Context>,
}

impl InputHandler {
    /// 创建新的 Input Handler
    pub fn new(
        state_path_tx: mpsc::Sender<StatePathMessage>,
        scheduler: Arc<MessageScheduler>,
    ) -> Self {
        Self {
            state_path_tx,
            scheduler,
            channel_strategies: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(tokio::sync::Mutex::new(HandlerStats::default())),
        }
    }

    /// 注册数据通道的分诊策略
    pub async fn register_channel_strategy(&self, channel_label: String, strategy: TriageStrategy) {
        let mut strategies = self.channel_strategies.write().await;
        strategies.insert(channel_label.clone(), strategy);
        info!("Registered triage strategy for channel: {}", channel_label);
    }

    /// 处理来自 WebRTC DataChannel 的消息
    /// 这是整个框架流量分诊的核心入口点
    pub async fn handle_datachannel_message(
        &self,
        channel: Arc<RTCDataChannel>,
        message_data: &[u8],
        source_actor: ActorId,
        target_actor: ActorId,
    ) -> ActorResult<()> {
        let channel_label = channel.label().to_string();
        let start_time = std::time::Instant::now();

        // 查找分诊策略
        let strategy = {
            let strategies = self.channel_strategies.read().await;
            strategies.get(&channel_label).cloned()
        };

        match strategy {
            Some(TriageStrategy::ChannelBased {
                control_channel,
                stream_channel_prefix,
            }) => {
                self.handle_channel_based_triage(
                    &channel_label,
                    &control_channel,
                    &stream_channel_prefix,
                    message_data,
                    source_actor,
                    target_actor,
                    start_time,
                )
                .await
            }
            Some(TriageStrategy::EnvelopeBased { unified_channel: _ }) => {
                self.handle_envelope_based_triage(
                    message_data,
                    source_actor,
                    target_actor,
                    start_time,
                )
                .await
            }
            None => {
                warn!(
                    "No triage strategy registered for channel: {}",
                    channel_label
                );
                Err(ActorError::RoutingFailed(format!(
                    "Unknown channel: {}",
                    channel_label
                )))
            }
        }
    }

    /// 处理基于 DataChannel 标签的物理隔离策略
    async fn handle_channel_based_triage(
        &self,
        channel_label: &str,
        control_channel: &str,
        stream_channel_prefix: &str,
        message_data: &[u8],
        source_actor: ActorId,
        target_actor: ActorId,
        start_time: std::time::Instant,
    ) -> ActorResult<()> {
        if channel_label == control_channel {
            // 这是一个状态类消息 - 发送到状态路径
            self.route_to_state_path(message_data, source_actor, target_actor)
                .await?;

            // 更新统计
            {
                let mut stats = self.stats.lock().await;
                stats.state_messages_processed += 1;
                stats.total_state_latency += start_time.elapsed().as_micros() as u64;
            }

            debug!(
                "Routed control message to state path from channel: {}",
                channel_label
            );
        } else if channel_label.starts_with(stream_channel_prefix) {
            // 这是流式数据 - 发送到快车道
            let stream_id = channel_label
                .strip_prefix(stream_channel_prefix)
                .unwrap_or(channel_label);

            self.route_to_fast_path(stream_id, message_data, source_actor, target_actor)
                .await?;

            // 更新统计
            {
                let mut stats = self.stats.lock().await;
                stats.stream_messages_processed += 1;
                stats.total_stream_latency += start_time.elapsed().as_micros() as u64;
            }

            debug!("Routed stream data to fast path for stream: {}", stream_id);
        } else {
            warn!("Unknown channel type: {}", channel_label);
            return Err(ActorError::RoutingFailed(format!(
                "Unknown channel type: {}",
                channel_label
            )));
        }

        Ok(())
    }

    /// 处理基于消息信封的逻辑隔离策略
    async fn handle_envelope_based_triage(
        &self,
        message_data: &[u8],
        source_actor: ActorId,
        target_actor: ActorId,
        start_time: std::time::Instant,
    ) -> ActorResult<()> {
        // 解析消息信封 (简化实现)
        if message_data.is_empty() {
            return Err(ActorError::InvalidMessage("Empty message".to_string()));
        }

        let message_type = message_data[0];
        let payload = &message_data[1..];

        match message_type {
            0x01 => {
                // 状态类消息
                self.route_to_state_path(payload, source_actor, target_actor)
                    .await?;

                // 更新统计
                {
                    let mut stats = self.stats.lock().await;
                    stats.state_messages_processed += 1;
                    stats.total_state_latency += start_time.elapsed().as_micros() as u64;
                }

                debug!("Routed envelope-based control message to state path");
            }
            0x02 => {
                // 流式数据 - 提取 stream_id (简化实现)
                let stream_id = "default_stream"; // 实际实现需要从 payload 中提取

                self.route_to_fast_path(stream_id, payload, source_actor, target_actor)
                    .await?;

                // 更新统计
                {
                    let mut stats = self.stats.lock().await;
                    stats.stream_messages_processed += 1;
                    stats.total_stream_latency += start_time.elapsed().as_micros() as u64;
                }

                debug!("Routed envelope-based stream data to fast path");
            }
            _ => {
                warn!("Unknown message type in envelope: 0x{:02x}", message_type);
                return Err(ActorError::InvalidMessage(format!(
                    "Unknown message type: 0x{:02x}",
                    message_type
                )));
            }
        }

        Ok(())
    }

    /// 路由消息到状态路径
    /// 这里是整个背压机制的"扳机" - 如果状态路径繁忙，这里会阻塞
    async fn route_to_state_path(
        &self,
        message_data: &[u8],
        source_actor: ActorId,
        target_actor: ActorId,
    ) -> ActorResult<()> {
        // 解析为内部消息格式
        let internal_msg = InternalMessage {
            payload: message_data.to_vec(),
            message_type: "parsed_from_wire".to_string(), // 实际实现需要消息类型识别
            priority: MessagePriority::Normal,            // 可以基于消息内容或来源动态确定
            is_stream: false,
            trace_id: uuid::Uuid::new_v4().to_string(),
            source_actor: Some(source_actor.clone()),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
        };

        // 创建上下文
        let context = Arc::new(Context::new(
            target_actor.clone(),
            Some(source_actor.clone()),
            crate::context::ActorSystemHandle::placeholder(), // 实际实现需要真实的 handle
        ));

        let state_msg = StatePathMessage {
            message: internal_msg,
            context,
        };

        // --- 这是整个背压机制的"扳机" ---
        // 如果状态路径繁忙，导致 state_path_tx 的 channel 满了，
        // 这里的 .send().await 操作将会暂停，不会立即返回。
        //
        // 这让 DataChannel 的接收缓冲区有机会被填满，
        // 从而触发 WebRTC 协议自身的背压机制。
        match self.state_path_tx.send(state_msg).await {
            Ok(_) => {
                debug!("Successfully sent message to state path");
                Ok(())
            }
            Err(_) => {
                error!("State path channel is closed");
                Err(ActorError::SystemShutdown)
            }
        }
    }

    /// 路由消息到快车道
    async fn route_to_fast_path(
        &self,
        stream_id: &str,
        message_data: &[u8],
        source_actor: ActorId,
        target_actor: ActorId,
    ) -> ActorResult<()> {
        // 创建上下文
        let context = Arc::new(Context::new(
            target_actor,
            Some(source_actor),
            crate::context::ActorSystemHandle::placeholder(), // 实际实现需要真实的 handle
        ));

        // 直接调用快车道处理，绕过队列
        self.scheduler
            .handle_fast_path_message(stream_id, message_data.to_vec(), context)
            .await
    }

    /// 获取处理器统计信息
    pub async fn get_stats(&self) -> HandlerStats {
        let stats = self.stats.lock().await;
        stats.clone()
    }

    /// 重置统计信息
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.lock().await;
        *stats = HandlerStats::default();
    }
}

/// Input Handler 统计信息
#[derive(Debug, Clone, Default)]
pub struct HandlerStats {
    pub state_messages_processed: u64,
    pub stream_messages_processed: u64,
    pub total_state_latency: u64,  // 微秒总计
    pub total_stream_latency: u64, // 微秒总计
    pub triage_errors: u64,
    pub backpressure_events: u64,
}

impl HandlerStats {
    /// 获取状态路径平均延迟 (微秒)
    pub fn avg_state_latency(&self) -> f64 {
        if self.state_messages_processed > 0 {
            self.total_state_latency as f64 / self.state_messages_processed as f64
        } else {
            0.0
        }
    }

    /// 获取快车道平均延迟 (微秒)
    pub fn avg_stream_latency(&self) -> f64 {
        if self.stream_messages_processed > 0 {
            self.total_stream_latency as f64 / self.stream_messages_processed as f64
        } else {
            0.0
        }
    }
}
