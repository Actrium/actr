//! 重新设计的 Actor 系统实现 - 符合文档要求的状态机模式

use crate::concurrent_handle::ConcurrentHandleManager;
use crate::context::{ActorSystemHandle, Context, SystemCommand};
use crate::error::{ActorError, ActorResult};
use crate::input_handler::InputHandler;
use crate::lifecycle::ILifecycle;
use crate::persistent_mailbox::{PersistentMailbox, PersistentMailboxConfig};
use crate::routing::{MessageScheduler, RouteProvider};
use crate::signaling::SignalingAdapter;
use crate::webrtc::WebRTCManager;
use shared_protocols::actor::ActorId;
use shared_protocols::signaling::{signaling_message::MessageType, SignalingMessage};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// 状态标记类型 - 用于类型安全的状态机模式
#[derive(Debug, Clone, Copy)]
pub struct Unattached;

#[derive(Debug, Clone, Copy)]
pub struct Attached;

/// Actor 系统运行时状态
#[derive(Debug, Clone, Copy, PartialEq)]
enum RuntimeState {
    Uninitialized,
    Starting,
    Running,
    Stopping,
    Stopped,
}

/// Actor 系统 - 框架的核心
///
/// 按照文档中的设计，ActorSystem 是"剧院"，为 Actor 提供所有必要的环境和支撑服务。
/// 使用状态机模式确保类型安全的 attach 和 start 流程。
pub struct ActorSystem<State> {
    /// 运行时状态
    runtime_state: Arc<RwLock<RuntimeState>>,
    /// 当前 Actor ID
    actor_id: ActorId,
    /// 信令适配器
    signaling: Option<Box<dyn SignalingAdapter>>,
    /// WebRTC 管理器
    webrtc_manager: Option<Arc<WebRTCManager>>,
    /// 消息调度器
    scheduler: Arc<MessageScheduler>,
    /// 系统句柄
    system_handle: Option<Arc<ActorSystemHandle>>,
    /// 系统命令接收器
    command_rx: Option<mpsc::UnboundedReceiver<SystemCommand>>,
    /// 输入处理器 - 负责初始分流
    input_handler: Option<Arc<InputHandler>>,
    /// 持久化邮箱配置
    persistence_config: Option<PersistentMailboxConfig>,
    /// 状态标记
    _state: PhantomData<State>,
}

/// Actor 系统与已附加的 Actor - 包含具体的 Actor 实例和路由信息
pub struct ActorSystemWithActor<T: ?Sized> {
    /// 核心系统
    system: ActorSystem<Attached>,
    /// 附加的 Actor 实例
    actor: Arc<T>,
    /// Actor 路由表
    #[allow(dead_code)]
    routes: HashMap<
        String,
        Box<
            dyn Fn(
                    Arc<Context>,
                    Vec<u8>,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = ActorResult<Vec<u8>>> + Send>,
                > + Send
                + Sync,
        >,
    >,
    /// 持久化邮箱 - 基于 WAL 的事务日志
    #[allow(dead_code)]
    persistent_mailbox: Option<Arc<PersistentMailbox>>,
    /// 并发句柄管理器 - Fast Path 生命周期管理
    #[allow(dead_code)]
    handle_manager: Arc<ConcurrentHandleManager>,
    /// Phantom data for type parameter
    _phantom: PhantomData<T>,
}

// ==================== 为 Unattached 状态实现方法 ====================

impl ActorSystem<Unattached> {
    /// 创建新的 Actor 系统（未附加状态）
    pub fn new(actor_id: ActorId) -> Self {
        let scheduler = Arc::new(MessageScheduler::new());

        Self {
            runtime_state: Arc::new(RwLock::new(RuntimeState::Uninitialized)),
            actor_id,
            signaling: None,
            webrtc_manager: None,
            scheduler,
            system_handle: None,
            command_rx: None,
            input_handler: None,
            persistence_config: None,
            _state: PhantomData,
        }
    }

    /// 设置信令适配器
    pub fn with_signaling(mut self, signaling: Box<dyn SignalingAdapter>) -> Self {
        self.signaling = Some(signaling);
        self
    }

    /// 配置持久化设置
    pub fn with_persistence(mut self, config: PersistentMailboxConfig) -> Self {
        self.persistence_config = Some(config);
        self
    }

    /// 简化的 attach 方法 - 通过 Actor 的 AttachableActor 实现自动推断适配器类型
    ///
    /// 这个方法提供更简洁的 API：`.attach(actor)` 而不需要显式指定类型参数
    pub fn attach<T>(self, actor: T) -> ActorSystemWithActor<T>
    where
        T: crate::routing::AttachableActor + Send + Sync + 'static,
        T::Adapter: RouteProvider<T> + 'static,
    {
        // 将 actor 包装为 Arc 并调用原有的方法
        self.attach_with_adapter::<T::Adapter, T>(Arc::new(actor))
    }

    /// 附加 Actor 实例 - 显式指定适配器类型（兼容性保留）
    ///
    /// 这是状态机模式的核心：只有 Unattached 系统可以调用 attach
    pub fn attach_with_adapter<A, T>(self, actor: Arc<T>) -> ActorSystemWithActor<T>
    where
        A: RouteProvider<T> + 'static,
        T: ?Sized + Send + Sync + 'static,
    {
        // 创建系统命令通道
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let system_handle = Arc::new(ActorSystemHandle::new(command_tx));

        // 创建并发句柄管理器
        let handle_manager = Arc::new(ConcurrentHandleManager::new());

        // 调用 RouteProvider 获取路由表
        let actor_routes = A::get_routes(actor.clone());
        let mut routes = HashMap::new();

        // 将路由添加到路由表
        for route in actor_routes {
            routes.insert(route.method_name, route.handler);
        }

        // 转换为 Attached 状态的系统
        let attached_system = ActorSystem {
            runtime_state: self.runtime_state,
            actor_id: self.actor_id,
            signaling: self.signaling,
            webrtc_manager: self.webrtc_manager,
            scheduler: self.scheduler,
            system_handle: Some(system_handle),
            command_rx: Some(command_rx),
            input_handler: self.input_handler,
            persistence_config: self.persistence_config,
            _state: PhantomData::<Attached>,
        };

        ActorSystemWithActor {
            system: attached_system,
            actor,
            routes,
            persistent_mailbox: None,
            handle_manager,
            _phantom: PhantomData,
        }
    }
}

// ==================== 为 ActorSystemWithActor 实现方法 ====================

impl<T: ?Sized> ActorSystemWithActor<T>
where
    T: ILifecycle + Send + Sync + 'static,
{
    /// 启动 Actor 系统 - 只有已附加的系统才能启动
    pub async fn start(mut self) -> ActorResult<()> {
        // 检查状态
        {
            let mut state = self.system.runtime_state.write().await;
            if *state != RuntimeState::Uninitialized {
                return Err(ActorError::Configuration(
                    "System already started".to_string(),
                ));
            }
            *state = RuntimeState::Starting;
        }

        info!(
            "Starting Actor System for actor {}",
            self.system.actor_id.serial_number
        );

        // 初始化 WebRTC 管理器
        let webrtc_manager = Arc::new(WebRTCManager::new()?);
        self.system.webrtc_manager = Some(webrtc_manager.clone());

        // 启动消息调度器
        self.system.scheduler.start().await?;

        // 连接信令服务器
        if let Some(ref mut signaling) = self.system.signaling {
            signaling.connect().await?;
            signaling.register_actor(&self.system.actor_id).await?;
            info!("Connected to signaling server");
        } else {
            warn!("No signaling adapter configured");
        }

        // 创建上下文
        let context = Arc::new(Context::new(
            self.system.actor_id.clone(),
            None,
            self.system.system_handle.clone().unwrap(),
        ));

        // 调用 Actor 的 on_start
        self.actor.on_start(context.clone()).await;

        // 更新状态为运行中
        {
            let mut state = self.system.runtime_state.write().await;
            *state = RuntimeState::Running;
        }

        info!("Actor System started successfully");

        // 启动主循环
        self.run().await?;

        Ok(())
    }

    /// 主运行循环
    async fn run(mut self) -> ActorResult<()> {
        // 获取信令消息接收器
        let mut signaling_rx = if let Some(ref mut signaling) = self.system.signaling {
            Some(signaling.receive_signals().await?)
        } else {
            None
        };

        let mut command_rx = self.system.command_rx.take().unwrap();

        info!("Entering main event loop");

        loop {
            // 检查系统状态
            {
                let state = self.system.runtime_state.read().await;
                if *state == RuntimeState::Stopping || *state == RuntimeState::Stopped {
                    break;
                }
            }

            tokio::select! {
                // 处理系统命令
                cmd = command_rx.recv() => {
                    match cmd {
                        Some(SystemCommand::Shutdown) => {
                            info!("Received shutdown command");
                            break;
                        },
                        Some(cmd) => {
                            self.handle_system_command(cmd).await;
                        },
                        None => {
                            warn!("System command channel closed");
                            break;
                        }
                    }
                },

                // 处理信令消息
                signaling_msg = async {
                    if let Some(ref mut rx) = signaling_rx {
                        rx.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    if let Some(msg) = signaling_msg {
                        self.handle_signaling_message(msg).await;
                    } else {
                        warn!("Signaling channel closed");
                    }
                },

                // 定期清理任务
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    self.perform_maintenance().await;
                }
            }
        }

        // 系统关闭流程
        self.shutdown().await?;
        Ok(())
    }

    /// 处理系统命令
    async fn handle_system_command(&self, command: SystemCommand) {
        match command {
            SystemCommand::SendMessage {
                target,
                payload: _,
                trace_id,
            } => {
                debug!(
                    "Sending message to {}, trace: {}",
                    target.serial_number, trace_id
                );
                // TODO: 通过 WebRTC 发送消息
            }
            SystemCommand::CallActor {
                target,
                payload: _,
                call_id,
                trace_id,
            } => {
                debug!(
                    "Calling actor {}, call: {}, trace: {}",
                    target.serial_number, call_id, trace_id
                );
                // TODO: 通过 WebRTC 发送调用请求
            }
            SystemCommand::ScheduleMessage {
                target,
                payload: _,
                delay,
                trace_id,
            } => {
                debug!(
                    "Scheduling message to {} with delay {:?}, trace: {}",
                    target.serial_number, delay, trace_id
                );
                // TODO: 实现延迟消息发送
            }
            SystemCommand::ConnectToPeer { target } => {
                debug!("Connecting to peer: {}", target.serial_number);
                if let Err(e) = self.connect_to_peer(&target).await {
                    error!("Failed to connect to peer {}: {}", target.serial_number, e);
                }
            }
            SystemCommand::DisconnectFromPeer { target } => {
                debug!("Disconnecting from peer: {}", target.serial_number);
                if let Err(e) = self.disconnect_from_peer(&target).await {
                    error!(
                        "Failed to disconnect from peer {}: {}",
                        target.serial_number, e
                    );
                }
            }
            SystemCommand::RouteRequest {
                payload: _,
                message_type,
                trace_id,
            } => {
                debug!(
                    "Routing request for message type: {}, trace: {}",
                    message_type, trace_id
                );
                // TODO: 实现基于消息类型的路由请求
            }
            SystemCommand::RouteNotify {
                payload: _,
                message_type,
                trace_id,
            } => {
                debug!(
                    "Routing notify for message type: {}, trace: {}",
                    message_type, trace_id
                );
                // TODO: 实现基于消息类型的路由通知
            }
            SystemCommand::RegisterFastPathCallback {
                stream_id,
                callback: _,
            } => {
                debug!("Registering Fast Path callback for stream: {}", stream_id);
                // TODO: 实现 Fast Path 回调注册
            }
            SystemCommand::UnregisterFastPathCallback { stream_id } => {
                debug!("Unregistering Fast Path callback for stream: {}", stream_id);
                // TODO: 实现 Fast Path 回调注销
            }
            SystemCommand::InvokeFastPathCallback { stream_id, data: _ } => {
                debug!("Invoking Fast Path callback for stream: {}", stream_id);
                // TODO: 实现 Fast Path 回调调用
            }
            SystemCommand::Shutdown => {
                // 这种情况在主循环中已经处理
            }
        }
    }

    /// 处理信令消息
    async fn handle_signaling_message(&self, message: SignalingMessage) {
        match message.message_type {
            Some(MessageType::NewActor(new_actor)) => {
                if let Some(actor_id) = new_actor.actor_id {
                    info!("Discovered new actor: {}", actor_id.serial_number);

                    // 调用 Actor 的 on_actor_discovered 回调
                    let context = Arc::new(Context::new(
                        self.system.actor_id.clone(),
                        None,
                        self.system.system_handle.clone().unwrap(),
                    ));

                    let should_connect = self.actor.on_actor_discovered(context, &actor_id).await;
                    if should_connect {
                        if let Err(e) = self.connect_to_peer(&actor_id).await {
                            error!(
                                "Failed to connect to discovered actor {}: {}",
                                actor_id.serial_number, e
                            );
                        }
                    }
                }
            }
            Some(MessageType::WebrtcSignal(signal)) => {
                debug!("Received WebRTC signal");
                self.handle_webrtc_signal(signal).await;
            }
            Some(MessageType::Error(error)) => {
                error!("Signaling error [{}]: {}", error.code, error.message);
            }
            _ => {
                debug!("Received unknown signaling message type");
            }
        }
    }

    /// 处理 WebRTC 信令
    async fn handle_webrtc_signal(&self, signal: shared_protocols::signaling::WebRtcSignal) {
        if let (Some(source), Some(target)) = (&signal.source_actor_id, &signal.target_actor_id) {
            // 检查是否是发送给我们的消息
            if target.serial_number != self.system.actor_id.serial_number {
                debug!("WebRTC signal not for us, ignoring");
                return;
            }

            info!("Processing WebRTC signal from {}", source.serial_number);
            // TODO: 实现完整的 WebRTC 信令处理
        }
    }

    /// 连接到对等 Actor
    async fn connect_to_peer(&self, target: &ActorId) -> ActorResult<()> {
        if let Some(ref webrtc_manager) = self.system.webrtc_manager {
            // 创建 WebRTC 连接
            let _peer_connection = webrtc_manager.create_connection(target, true).await?;
            let _offer = webrtc_manager.create_offer(target).await?;

            // TODO: 通过信令发送 Offer
            info!("Initiated connection to peer {}", target.serial_number);
        }

        Ok(())
    }

    /// 断开与对等 Actor 的连接
    async fn disconnect_from_peer(&self, target: &ActorId) -> ActorResult<()> {
        if let Some(ref webrtc_manager) = self.system.webrtc_manager {
            webrtc_manager.close_connection(target).await?;
            info!("Disconnected from peer {}", target.serial_number);
        }

        Ok(())
    }

    /// 执行维护任务
    async fn perform_maintenance(&self) {
        debug!("Performing system maintenance");

        // 清理过期的快车道处理器
        let cleanup_duration = std::time::Duration::from_secs(300); // 5分钟
        self.system
            .scheduler
            .cleanup_expired_handlers(cleanup_duration)
            .await;
    }

    /// 系统关闭流程
    async fn shutdown(mut self) -> ActorResult<()> {
        info!("Shutting down Actor System");

        // 更新状态
        {
            let mut state = self.system.runtime_state.write().await;
            *state = RuntimeState::Stopping;
        }

        // 调用 Actor 的 on_stop
        let context = Arc::new(Context::new(
            self.system.actor_id.clone(),
            None,
            self.system.system_handle.clone().unwrap(),
        ));
        self.actor.on_stop(context).await;

        // 停止消息调度器
        self.system.scheduler.stop().await;

        // 断开信令连接
        if let Some(ref mut signaling) = self.system.signaling {
            let _ = signaling.disconnect().await;
        }

        // 关闭所有 WebRTC 连接
        if let Some(ref webrtc_manager) = self.system.webrtc_manager {
            let stats = webrtc_manager.get_connection_stats().await;
            for (_, stat) in stats {
                let _ = webrtc_manager.close_connection(&stat.target_actor).await;
            }
        }

        // 更新最终状态
        {
            let mut state = self.system.runtime_state.write().await;
            *state = RuntimeState::Stopped;
        }

        info!("Actor System shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::routing::Route;
    use shared_protocols::actor::ActorTypeCode;

    #[derive(Default)]
    struct TestActor;

    #[async_trait]
    impl ILifecycle for TestActor {
        async fn on_start(&self, _ctx: Arc<Context>) {
            // Test implementation
        }

        async fn on_stop(&self, _ctx: Arc<Context>) {
            // Test implementation
        }

        async fn on_actor_discovered(&self, _ctx: Arc<Context>, _actor_id: &ActorId) -> bool {
            false
        }
    }

    // 为测试实现 RouteProvider
    struct TestActorAdapter;

    impl RouteProvider<dyn ILifecycle> for TestActorAdapter {
        fn get_routes(_actor: Arc<dyn ILifecycle>) -> Vec<Route> {
            vec![]
        }
    }

    #[tokio::test]
    async fn test_actor_system_state_machine() {
        let actor_id = ActorId::new(1001, ActorTypeCode::Authenticated, "test_actor".to_string());
        let actor = Arc::new(TestActor::default());

        // 测试状态机流程
        let unattached_system = ActorSystem::new(actor_id);
        let attached_system = unattached_system.attach::<TestActorAdapter, _>(actor);

        // 注意：实际测试中 start() 会运行主循环，这里只是验证类型
        // attached_system.start().await.unwrap();

        // 测试编译时保证
        // 下面的代码应该编译失败：
        // let unattached = ActorSystem::new(actor_id);
        // unattached.start().await; // ❌ 编译错误：Unattached 没有 start 方法

        // unattached.attach(actor1).attach(actor2); // ❌ 编译错误：Attached 没有 attach 方法
    }
}
