//! 消息路由和调度系统

use crate::context::Context;
use crate::error::{ActorError, ActorResult};
use crate::messaging::{InternalMessage, MessagePriority, MessageRouter};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// 路由处理器 - 将消息路由到对应的处理函数
pub struct Route {
    pub method_name: String,
    pub handler: Box<
        dyn Fn(
                Arc<Context>,
                Vec<u8>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = crate::error::ActorResult<Vec<u8>>> + Send>,
            > + Send
            + Sync,
    >,
}

/// 路由提供者 trait - 用于生成的 ActorAdapter 实现
pub trait RouteProvider<T: ?Sized> {
    /// 获取该 Actor 的所有路由
    fn get_routes(actor: Arc<T>) -> Vec<Route>;
}

/// 可附加的 Actor trait - 支持简化的 attach API
pub trait AttachableActor {
    /// 关联的路由适配器类型
    type Adapter;
}

/// 消息调度器 - 实现双路径处理模型
pub struct MessageScheduler {
    /// 高优先级队列（系统关键操作）
    high_priority_queue: Arc<Mutex<VecDeque<ScheduledMessage>>>,
    /// 普通优先级队列（一般业务逻辑）
    normal_priority_queue: Arc<Mutex<VecDeque<ScheduledMessage>>>,
    /// 快车道注册表（流式数据处理）
    fast_path_registry: Arc<RwLock<HashMap<String, FastPathHandler>>>,
    /// 消息路由器
    message_router: Arc<RwLock<MessageRouter>>,
    /// 调度器状态
    running: Arc<AtomicBool>,
    /// 性能统计
    stats: Arc<Mutex<SchedulerStats>>,
}

impl MessageScheduler {
    /// 创建新的消息调度器
    pub fn new() -> Self {
        Self {
            high_priority_queue: Arc::new(Mutex::new(VecDeque::new())),
            normal_priority_queue: Arc::new(Mutex::new(VecDeque::new())),
            fast_path_registry: Arc::new(RwLock::new(HashMap::new())),
            message_router: Arc::new(RwLock::new(MessageRouter::new())),
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(SchedulerStats::default())),
        }
    }

    /// 启动调度器
    pub async fn start(&self) -> ActorResult<()> {
        if self.running.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ActorError::Configuration(
                "Scheduler already running".to_string(),
            ));
        }

        self.running
            .store(true, std::sync::atomic::Ordering::Release);
        info!("Message scheduler started");

        // 启动状态路径处理循环
        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.state_path_loop().await;
        });

        Ok(())
    }

    /// 停止调度器
    pub async fn stop(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::Release);
        info!("Message scheduler stopped");
    }

    /// 调度消息到状态路径
    #[allow(dead_code)]
    pub(crate) async fn schedule_state_message(
        &self,
        message: InternalMessage,
        context: Arc<Context>,
    ) -> ActorResult<()> {
        let scheduled = ScheduledMessage {
            message,
            context,
            scheduled_at: Instant::now(),
        };

        // 根据优先级选择队列
        match scheduled.message.priority {
            MessagePriority::Critical | MessagePriority::High => {
                let mut queue = self.high_priority_queue.lock().await;
                queue.push_back(scheduled);
                debug!("Scheduled high priority message");
            }
            MessagePriority::Normal | MessagePriority::Low => {
                let mut queue = self.normal_priority_queue.lock().await;
                queue.push_back(scheduled);
                debug!("Scheduled normal priority message");
            }
        }

        // 更新统计
        {
            let mut stats = self.stats.lock().await;
            stats.total_messages_scheduled += 1;
            stats.state_path_messages += 1;
        }

        Ok(())
    }

    /// 处理快车道消息
    pub async fn handle_fast_path_message(
        &self,
        stream_id: &str,
        data: Vec<u8>,
        context: Arc<Context>,
    ) -> ActorResult<()> {
        let start_time = Instant::now();

        // 查找快车道处理器
        let handler = {
            let registry = self.fast_path_registry.read().await;
            registry.get(stream_id).cloned()
        };

        match handler {
            Some(handler) => {
                // 直接调用处理器，绕过队列
                if let Err(e) = (handler.callback)(data, context).await {
                    error!("Fast path handler error: {}", e);
                }

                // 更新统计
                {
                    let mut stats = self.stats.lock().await;
                    stats.total_messages_processed += 1;
                    stats.fast_path_messages += 1;
                    stats.fast_path_avg_latency = (stats.fast_path_avg_latency
                        * (stats.fast_path_messages - 1) as f64
                        + start_time.elapsed().as_micros() as f64)
                        / stats.fast_path_messages as f64;
                }

                debug!(
                    "Processed fast path message for stream {} in {:?}",
                    stream_id,
                    start_time.elapsed()
                );
            }
            None => {
                warn!("No fast path handler registered for stream: {}", stream_id);
                return Err(ActorError::RoutingFailed(format!(
                    "No handler for stream {}",
                    stream_id
                )));
            }
        }

        Ok(())
    }

    /// 注册快车道处理器
    pub async fn register_fast_path_handler<F, Fut>(
        &self,
        stream_id: String,
        handler: F,
    ) -> ActorResult<()>
    where
        F: Fn(Vec<u8>, Arc<Context>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ActorResult<()>> + Send + 'static,
    {
        let handler = FastPathHandler {
            stream_id: stream_id.clone(),
            callback: Arc::new(move |data, ctx| {
                let future = handler(data, ctx);
                Box::pin(future)
            }),
            registered_at: Instant::now(),
        };

        {
            let mut registry = self.fast_path_registry.write().await;
            registry.insert(stream_id.clone(), handler);
        }

        info!("Registered fast path handler for stream: {}", stream_id);
        Ok(())
    }

    /// 注销快车道处理器
    pub async fn unregister_fast_path_handler(&self, stream_id: &str) -> ActorResult<()> {
        {
            let mut registry = self.fast_path_registry.write().await;
            registry.remove(stream_id);
        }

        info!("Unregistered fast path handler for stream: {}", stream_id);
        Ok(())
    }

    /// 获取消息路由器
    #[allow(dead_code)]
    pub(crate) async fn get_message_router(&self) -> Arc<RwLock<MessageRouter>> {
        self.message_router.clone()
    }

    /// 状态路径处理循环 - 使用双通道偏向性调度
    /// 实现文档中描述的 tokio::select! biased 优先级机制
    async fn state_path_loop(&self) {
        info!("Starting state path processing loop with biased scheduling");

        // 创建高优和普通优先级的 mpsc 通道
        let (high_priority_tx, mut high_priority_rx) = mpsc::channel::<ScheduledMessage>(128);
        let (normal_priority_tx, mut normal_priority_rx) = mpsc::channel::<ScheduledMessage>(64);

        // 启动队列监控任务，将队列中的消息转发到 channel
        let high_queue_monitor = {
            let queue = self.high_priority_queue.clone();
            let tx = high_priority_tx.clone();
            tokio::spawn(async move {
                loop {
                    if let Some(message) = {
                        let mut q = queue.lock().await;
                        q.pop_front()
                    } {
                        if tx.send(message).await.is_err() {
                            break; // channel 已关闭
                        }
                    } else {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            })
        };

        let normal_queue_monitor = {
            let queue = self.normal_priority_queue.clone();
            let tx = normal_priority_tx.clone();
            tokio::spawn(async move {
                loop {
                    if let Some(message) = {
                        let mut q = queue.lock().await;
                        q.pop_front()
                    } {
                        if tx.send(message).await.is_err() {
                            break; // channel 已关闭
                        }
                    } else {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            })
        };

        // 创建关闭信号
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        let running = self.running.clone();
        tokio::spawn(async move {
            while running.load(std::sync::atomic::Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let _ = shutdown_tx.send(()).await;
        });

        // **核心调度循环 - 实现文档中的 tokio::select! biased 机制**
        loop {
            tokio::select! {
                // biased 属性确保按顺序检查分支，实现优先级调度
                biased;

                // 最高优先级：高优先级消息
                Some(high_prio_msg) = high_priority_rx.recv() => {
                    debug!("Processing HIGH priority message");
                    self.process_scheduled_message(high_prio_msg).await;
                },

                // 次优先级：普通优先级消息
                Some(normal_prio_msg) = normal_priority_rx.recv() => {
                    debug!("Processing NORMAL priority message");
                    self.process_scheduled_message(normal_prio_msg).await;
                },

                // 系统关闭信号
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal, stopping state path loop");
                    break;
                },

                // 如果所有通道都空，短暂等待
                else => {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                },
            }
        }

        // 清理监控任务
        high_queue_monitor.abort();
        normal_queue_monitor.abort();

        info!("State path processing loop stopped");
    }

    /// 处理调度的消息
    async fn process_scheduled_message(&self, scheduled: ScheduledMessage) {
        let start_time = Instant::now();
        let message_type = scheduled.message.message_type.clone();

        // 通过消息路由器处理消息
        let router = self.message_router.read().await;
        let result = router
            .route_raw_message(
                &scheduled.message.message_type,
                &scheduled.message.payload,
                scheduled.message.is_stream,
                scheduled.context,
            )
            .await;

        match result {
            Ok(_) => {
                debug!("Successfully processed message type: {}", message_type);
            }
            Err(e) => {
                error!("Failed to process message type {}: {}", message_type, e);
            }
        }

        // 更新统计
        {
            let mut stats = self.stats.lock().await;
            stats.total_messages_processed += 1;

            let latency = start_time.elapsed().as_micros() as f64;
            stats.state_path_avg_latency = (stats.state_path_avg_latency
                * (stats.total_messages_processed - 1) as f64
                + latency)
                / stats.total_messages_processed as f64;
        }
    }

    /// 获取调度器统计信息
    pub async fn get_stats(&self) -> SchedulerStats {
        let stats = self.stats.lock().await;
        stats.clone()
    }

    /// 清理过期的快车道处理器
    pub async fn cleanup_expired_handlers(&self, max_age: Duration) {
        let now = Instant::now();
        let mut expired_handlers = Vec::new();

        {
            let registry = self.fast_path_registry.read().await;
            for (stream_id, handler) in registry.iter() {
                if now.duration_since(handler.registered_at) > max_age {
                    expired_handlers.push(stream_id.clone());
                }
            }
        }

        if !expired_handlers.is_empty() {
            let mut registry = self.fast_path_registry.write().await;
            for stream_id in expired_handlers {
                registry.remove(&stream_id);
                debug!("Removed expired fast path handler: {}", stream_id);
            }
        }
    }
}

impl Clone for MessageScheduler {
    fn clone(&self) -> Self {
        Self {
            high_priority_queue: self.high_priority_queue.clone(),
            normal_priority_queue: self.normal_priority_queue.clone(),
            fast_path_registry: self.fast_path_registry.clone(),
            message_router: self.message_router.clone(),
            running: self.running.clone(),
            stats: self.stats.clone(),
        }
    }
}

/// 调度的消息
#[derive(Debug)]
struct ScheduledMessage {
    message: InternalMessage,
    context: Arc<Context>,
    #[allow(dead_code)]
    scheduled_at: Instant,
}

/// 快车道处理器
#[derive(Clone)]
struct FastPathHandler {
    #[allow(dead_code)]
    stream_id: String,
    callback: Arc<
        dyn Fn(
                Vec<u8>,
                Arc<Context>,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = ActorResult<()>> + Send>>
            + Send
            + Sync,
    >,
    registered_at: Instant,
}

/// 调度器性能统计
#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    pub total_messages_scheduled: u64,
    pub total_messages_processed: u64,
    pub state_path_messages: u64,
    pub fast_path_messages: u64,
    pub state_path_avg_latency: f64, // 微秒
    pub fast_path_avg_latency: f64,  // 微秒
    pub queue_high_size: usize,
    pub queue_normal_size: usize,
    pub fast_path_handlers: usize,
}
