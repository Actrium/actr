//! 快车道并发句柄 - 生命周期与并发安全
//!
//! 实现"并发句柄"模式，允许快车道回调安全地与 State Path 交互。
//! 句柄内部将所有状态变更操作封装成向 State Path 发送消息的异步方法。

use crate::context::Context;
use crate::error::{ActorError, ActorResult};
use crate::messaging::{InternalMessage, MessagePriority};
use shared_protocols::actor::ActorId;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error};
use uuid::Uuid;

/// 并发句柄 - Fast Path 与 State Path 安全交互的桥梁
///
/// 这个句柄是 Actor 的一个线程安全的代理，它将所有需要修改核心状态的操作，
/// 都封装成向 State Path 发送消息的异步方法。
#[derive(Clone)]
pub struct ConcurrentHandle {
    /// 目标 Actor ID
    actor_id: ActorId,
    /// 原始上下文信息
    context_info: ContextInfo,
    /// 向 State Path 发送消息的通道
    state_path_tx: mpsc::Sender<StatePathMessage>,
    /// 句柄创建时间 (用于生命周期管理)
    created_at: std::time::Instant,
    /// 句柄 ID (用于调试和追踪)
    handle_id: String,
}

/// 上下文信息快照 (从原始 Context 中提取的只读信息)
#[derive(Debug, Clone)]
struct ContextInfo {
    caller_id: Option<ActorId>,
    trace_id: String,
}

// 使用 input_handler 模块中已定义的 StatePathMessage
pub use crate::input_handler::StatePathMessage;

impl ConcurrentHandle {
    /// 创建新的并发句柄
    ///
    /// 这个方法通常由 Actor 在注册 Fast Path 回调时调用
    pub fn new(
        actor_id: ActorId,
        original_context: Arc<Context>,
        state_path_tx: mpsc::Sender<StatePathMessage>,
    ) -> Self {
        let handle_id = format!("handle-{}", Uuid::new_v4().simple());

        let context_info = ContextInfo {
            caller_id: original_context.caller_id.clone(),
            trace_id: original_context.trace_id.clone(),
        };

        debug!(
            "Created concurrent handle {} for actor {:?}",
            handle_id, actor_id
        );

        Self {
            actor_id,
            context_info,
            state_path_tx,
            created_at: std::time::Instant::now(),
            handle_id,
        }
    }

    /// 处理文件块 - 示例方法
    ///
    /// 这个方法展示了如何在并发句柄中安全地处理需要状态变更的操作
    pub async fn handle_file_chunk(&self, stream_id: String, chunk: Vec<u8>) -> ActorResult<()> {
        // 1. 纯数据处理，可以在当前线程执行 (无状态操作)
        let processed_chunk = self.process_chunk_data(chunk);

        // 2. 需要状态变更的操作 - 封装成消息发送给 State Path
        let chunk_processed_msg = ChunkProcessedMessage {
            stream_id,
            processed_data: processed_chunk.data,
            chunk_size: processed_chunk.size,
            processing_time_ms: processed_chunk.processing_time_ms,
        };

        self.send_to_state_path(
            "handle_chunk_processed",
            chunk_processed_msg,
            MessagePriority::Normal,
        )
        .await
    }

    /// 处理流错误 - 示例方法
    pub async fn handle_stream_error(
        &self,
        stream_id: String,
        error_code: u32,
        error_message: String,
    ) -> ActorResult<()> {
        let stream_error_msg = StreamErrorMessage {
            stream_id,
            error_code,
            error_message,
            timestamp: std::time::SystemTime::now(),
        };

        // 流错误是高优先级事件，应该立即处理
        self.send_to_state_path(
            "handle_stream_error",
            stream_error_msg,
            MessagePriority::High,
        )
        .await
    }

    /// 更新流统计信息 - 示例方法
    pub async fn update_stream_stats(
        &self,
        stream_id: String,
        bytes_processed: u64,
        latency_ms: u32,
    ) -> ActorResult<()> {
        let stats_update_msg = StreamStatsUpdateMessage {
            stream_id,
            bytes_processed,
            latency_ms,
            timestamp: std::time::SystemTime::now(),
        };

        // 统计更新是低优先级操作
        self.send_to_state_path(
            "update_stream_stats",
            stats_update_msg,
            MessagePriority::Low,
        )
        .await
    }

    /// 请求关闭流 - 示例方法
    pub async fn request_stream_close(&self, stream_id: String, reason: String) -> ActorResult<()> {
        let close_request_msg = StreamCloseRequestMessage {
            stream_id,
            reason,
            requested_by: self.handle_id.clone(),
            timestamp: std::time::SystemTime::now(),
        };

        // 流关闭请求是高优先级操作
        self.send_to_state_path(
            "request_stream_close",
            close_request_msg,
            MessagePriority::High,
        )
        .await
    }

    /// 获取句柄创建时间 (只读操作，线程安全)
    pub fn created_at(&self) -> std::time::Instant {
        self.created_at
    }

    /// 获取句柄 ID (只读操作，线程安全)
    pub fn handle_id(&self) -> &str {
        &self.handle_id
    }

    /// 获取目标 Actor ID (只读操作，线程安全)
    pub fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// 检查句柄是否过期
    pub fn is_expired(&self, max_age: std::time::Duration) -> bool {
        self.created_at.elapsed() > max_age
    }

    // === 私有辅助方法 ===

    /// 通用的 State Path 消息发送方法
    async fn send_to_state_path<T>(
        &self,
        method_name: &str,
        message: T,
        priority: MessagePriority,
    ) -> ActorResult<()>
    where
        T: serde::Serialize,
    {
        // 序列化消息
        let payload = serde_json::to_vec(&message).map_err(|e| {
            ActorError::SerializationFailed(format!("Failed to serialize message: {}", e))
        })?;

        // 创建内部消息
        let internal_msg = InternalMessage {
            payload,
            message_type: method_name.to_string(),
            priority,
            is_stream: false, // 来自并发句柄的消息总是状态类消息
            trace_id: self.context_info.trace_id.clone(),
            source_actor: self.context_info.caller_id.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
        };

        // 重建上下文 (使用快照的信息)
        let context = Arc::new(Context::with_trace_id(
            self.actor_id.clone(),
            self.context_info.caller_id.clone(),
            self.context_info.trace_id.clone(),
            crate::context::ActorSystemHandle::placeholder(), // 实际实现需要真实的 handle
        ));

        // 创建 State Path 消息
        let state_msg = StatePathMessage {
            message: internal_msg,
            context,
        };

        // 发送给 State Path
        match self.state_path_tx.send(state_msg).await {
            Ok(_) => {
                debug!(
                    "Concurrent handle {} sent {} message to state path",
                    self.handle_id, method_name
                );
                Ok(())
            }
            Err(_) => {
                error!(
                    "Failed to send message from concurrent handle {}: channel closed",
                    self.handle_id
                );
                Err(ActorError::SystemShutdown)
            }
        }
    }

    /// 处理块数据 (纯数据处理，无状态变更)
    fn process_chunk_data(&self, chunk: Vec<u8>) -> ProcessedChunk {
        let start_time = std::time::Instant::now();

        // 这里可以进行各种数据处理：
        // - 解压缩
        // - 解密
        // - 格式转换
        // - 校验和计算
        // 等等...

        // 示例：简单的数据复制和统计
        let processed_data = chunk.clone();
        let size = chunk.len();
        let processing_time_ms = start_time.elapsed().as_millis() as u32;

        ProcessedChunk {
            data: processed_data,
            size,
            processing_time_ms,
        }
    }
}

// === 消息定义 ===

/// 处理后的块数据
struct ProcessedChunk {
    data: Vec<u8>,
    size: usize,
    processing_time_ms: u32,
}

/// 块处理完成消息
#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct ChunkProcessedMessage {
    stream_id: String,
    processed_data: Vec<u8>,
    chunk_size: usize,
    processing_time_ms: u32,
}

/// 流错误消息
#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct StreamErrorMessage {
    stream_id: String,
    error_code: u32,
    error_message: String,
    timestamp: std::time::SystemTime,
}

/// 流统计更新消息
#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct StreamStatsUpdateMessage {
    stream_id: String,
    bytes_processed: u64,
    latency_ms: u32,
    timestamp: std::time::SystemTime,
}

/// 流关闭请求消息
#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct StreamCloseRequestMessage {
    stream_id: String,
    reason: String,
    requested_by: String,
    timestamp: std::time::SystemTime,
}

/// 并发句柄管理器 - 用于生命周期管理
pub struct ConcurrentHandleManager {
    /// 活跃的句柄映射
    active_handles: Arc<tokio::sync::RwLock<std::collections::HashMap<String, ConcurrentHandle>>>,
    /// 清理任务句柄
    cleanup_task: Option<tokio::task::JoinHandle<()>>,
}

impl ConcurrentHandleManager {
    /// 创建新的句柄管理器
    pub fn new() -> Self {
        Self {
            active_handles: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            cleanup_task: None,
        }
    }

    /// 注册新的并发句柄
    pub async fn register_handle(&self, handle: ConcurrentHandle) {
        let handle_id = handle.handle_id().to_string();
        let mut handles = self.active_handles.write().await;
        handles.insert(handle_id.clone(), handle);
        debug!("Registered concurrent handle: {}", handle_id);
    }

    /// 注销并发句柄
    pub async fn unregister_handle(&self, handle_id: &str) {
        let mut handles = self.active_handles.write().await;
        if handles.remove(handle_id).is_some() {
            debug!("Unregistered concurrent handle: {}", handle_id);
        }
    }

    /// 清理过期的句柄
    pub async fn cleanup_expired_handles(&self, max_age: std::time::Duration) {
        let mut handles = self.active_handles.write().await;
        let mut expired_handles = Vec::new();

        for (handle_id, handle) in handles.iter() {
            if handle.is_expired(max_age) {
                expired_handles.push(handle_id.clone());
            }
        }

        for handle_id in expired_handles {
            handles.remove(&handle_id);
            debug!("Cleaned up expired concurrent handle: {}", handle_id);
        }
    }

    /// 启动自动清理任务
    pub fn start_cleanup_task(
        &mut self,
        cleanup_interval: std::time::Duration,
        max_handle_age: std::time::Duration,
    ) {
        let handles = self.active_handles.clone();

        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(cleanup_interval);

            loop {
                interval.tick().await;

                // 清理过期句柄
                let mut handles_guard = handles.write().await;
                let mut expired_handles = Vec::new();

                for (handle_id, handle) in handles_guard.iter() {
                    if handle.is_expired(max_handle_age) {
                        expired_handles.push(handle_id.clone());
                    }
                }

                for handle_id in expired_handles {
                    handles_guard.remove(&handle_id);
                    debug!("Auto-cleaned expired concurrent handle: {}", handle_id);
                }
            }
        });

        self.cleanup_task = Some(task);
    }

    /// 停止清理任务
    pub async fn stop_cleanup_task(&mut self) {
        if let Some(task) = self.cleanup_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    /// 获取活跃句柄数量
    pub async fn active_handle_count(&self) -> usize {
        let handles = self.active_handles.read().await;
        handles.len()
    }
}
