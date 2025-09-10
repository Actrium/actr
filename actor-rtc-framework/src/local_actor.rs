//! LocalActor implementation - 本地 Actor 的完整实现
//!
//! LocalActor 表示在当前进程中运行的 Actor，拥有完整的生命周期控制和状态管理能力

use crate::context::Context;
use crate::error::{ActorError, ActorResult};
use shared_protocols::actor::ActorId;
use std::sync::Arc;
use tracing::{debug, info};

/// 本地 Actor trait - 定义本地 Actor 的核心行为
#[async_trait::async_trait]
pub trait LocalActor: Send + Sync + 'static {
    /// Actor 初始化
    async fn initialize(&self, ctx: Arc<Context>) -> ActorResult<()>;

    /// Actor 启动
    async fn start(&self, ctx: Arc<Context>) -> ActorResult<()>;

    /// Actor 停止
    async fn stop(&self, ctx: Arc<Context>) -> ActorResult<()>;

    /// 处理状态路径消息
    async fn handle_state_message(&self, ctx: Arc<Context>, message: Vec<u8>) -> ActorResult<()>;

    /// 获取 Actor ID
    fn get_actor_id(&self) -> &ActorId;

    /// 获取 Actor 类型名称
    fn get_type_name(&self) -> &str;
}

/// 本地 Actor 容器 - 管理本地 Actor 的生命周期
pub struct LocalActorContainer<T: LocalActor> {
    /// Actor 实例
    actor: Arc<T>,
    /// Actor 上下文
    context: Arc<Context>,
    /// 生命周期状态
    lifecycle_state: LifecycleState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LifecycleState {
    Created,
    Initialized,
    Started,
    Stopped,
}

impl<T: LocalActor> LocalActorContainer<T> {
    /// 创建新的本地 Actor 容器
    pub fn new(actor: Arc<T>, context: Arc<Context>) -> Self {
        Self {
            actor,
            context,
            lifecycle_state: LifecycleState::Created,
        }
    }

    /// 初始化 Actor
    pub async fn initialize(&mut self) -> ActorResult<()> {
        if self.lifecycle_state != LifecycleState::Created {
            return Err(ActorError::InvalidState(format!(
                "Actor already initialized or in invalid state: {:?}",
                self.lifecycle_state
            )));
        }

        info!(
            "Initializing local actor: {}",
            self.actor.get_actor_id().serial_number
        );

        self.actor.initialize(self.context.clone()).await?;
        self.lifecycle_state = LifecycleState::Initialized;

        debug!(
            "Local actor initialized: {}",
            self.actor.get_actor_id().serial_number
        );

        Ok(())
    }

    /// 启动 Actor
    pub async fn start(&mut self) -> ActorResult<()> {
        if self.lifecycle_state != LifecycleState::Initialized {
            return Err(ActorError::InvalidState(format!(
                "Actor must be initialized before starting: {:?}",
                self.lifecycle_state
            )));
        }

        info!(
            "Starting local actor: {}",
            self.actor.get_actor_id().serial_number
        );

        self.actor.start(self.context.clone()).await?;
        self.lifecycle_state = LifecycleState::Started;

        debug!(
            "Local actor started: {}",
            self.actor.get_actor_id().serial_number
        );

        Ok(())
    }

    /// 停止 Actor
    pub async fn stop(&mut self) -> ActorResult<()> {
        if self.lifecycle_state != LifecycleState::Started {
            return Err(ActorError::InvalidState(format!(
                "Actor not started: {:?}",
                self.lifecycle_state
            )));
        }

        info!(
            "Stopping local actor: {}",
            self.actor.get_actor_id().serial_number
        );

        self.actor.stop(self.context.clone()).await?;
        self.lifecycle_state = LifecycleState::Stopped;

        debug!(
            "Local actor stopped: {}",
            self.actor.get_actor_id().serial_number
        );

        Ok(())
    }

    /// 处理消息
    pub async fn handle_message(&self, message: Vec<u8>) -> ActorResult<()> {
        if self.lifecycle_state != LifecycleState::Started {
            return Err(ActorError::InvalidState(format!(
                "Actor not in running state: {:?}",
                self.lifecycle_state
            )));
        }

        self.actor
            .handle_state_message(self.context.clone(), message)
            .await
    }

    /// 获取 Actor 引用
    pub fn get_actor(&self) -> &Arc<T> {
        &self.actor
    }

    /// 获取上下文引用
    pub fn get_context(&self) -> &Arc<Context> {
        &self.context
    }

    /// 获取生命周期状态
    #[allow(dead_code)]
    pub(crate) fn get_lifecycle_state(&self) -> LifecycleState {
        self.lifecycle_state
    }

    /// 检查 Actor 是否在运行中
    pub fn is_running(&self) -> bool {
        self.lifecycle_state == LifecycleState::Started
    }
}

/// 本地 Actor 管理器 - 管理多个本地 Actor
pub struct LocalActorManager {
    /// 注册的 Actor 容器 (使用 serial_number 作为键)
    actors: std::collections::HashMap<u64, Box<dyn LocalActorContainerErased>>,
}

/// 类型擦除的本地 Actor 容器 trait
#[async_trait::async_trait]
trait LocalActorContainerErased: Send + Sync {
    async fn initialize(&mut self) -> ActorResult<()>;
    async fn start(&mut self) -> ActorResult<()>;
    async fn stop(&mut self) -> ActorResult<()>;
    async fn handle_message(&self, message: Vec<u8>) -> ActorResult<()>;
    fn get_actor_id(&self) -> ActorId;
    fn is_running(&self) -> bool;
}

#[async_trait::async_trait]
impl<T: LocalActor> LocalActorContainerErased for LocalActorContainer<T> {
    async fn initialize(&mut self) -> ActorResult<()> {
        self.initialize().await
    }

    async fn start(&mut self) -> ActorResult<()> {
        self.start().await
    }

    async fn stop(&mut self) -> ActorResult<()> {
        self.stop().await
    }

    async fn handle_message(&self, message: Vec<u8>) -> ActorResult<()> {
        self.handle_message(message).await
    }

    fn get_actor_id(&self) -> ActorId {
        self.actor.get_actor_id().clone()
    }

    fn is_running(&self) -> bool {
        self.is_running()
    }
}

impl LocalActorManager {
    /// 创建新的本地 Actor 管理器
    pub fn new() -> Self {
        Self {
            actors: std::collections::HashMap::new(),
        }
    }

    /// 注册本地 Actor
    pub fn register_actor<T: LocalActor>(
        &mut self,
        actor: Arc<T>,
        context: Arc<Context>,
    ) -> ActorResult<()> {
        let actor_id = actor.get_actor_id();
        let serial_number = actor_id.serial_number;
        let container = LocalActorContainer::new(actor, context);

        if self.actors.contains_key(&serial_number) {
            return Err(ActorError::InvalidState(format!(
                "Actor already registered: {}",
                serial_number
            )));
        }

        self.actors.insert(serial_number, Box::new(container));
        Ok(())
    }

    /// 初始化指定的 Actor
    pub async fn initialize_actor(&mut self, actor_id: &ActorId) -> ActorResult<()> {
        let serial_number = actor_id.serial_number;
        match self.actors.get_mut(&serial_number) {
            Some(container) => container.initialize().await,
            None => Err(ActorError::ActorNotFound {
                actor_id: format!("{}", serial_number),
            }),
        }
    }

    /// 启动指定的 Actor
    pub async fn start_actor(&mut self, actor_id: &ActorId) -> ActorResult<()> {
        let serial_number = actor_id.serial_number;
        match self.actors.get_mut(&serial_number) {
            Some(container) => container.start().await,
            None => Err(ActorError::ActorNotFound {
                actor_id: format!("{}", serial_number),
            }),
        }
    }

    /// 停止指定的 Actor
    pub async fn stop_actor(&mut self, actor_id: &ActorId) -> ActorResult<()> {
        let serial_number = actor_id.serial_number;
        match self.actors.get_mut(&serial_number) {
            Some(container) => container.stop().await,
            None => Err(ActorError::ActorNotFound {
                actor_id: format!("{}", serial_number),
            }),
        }
    }

    /// 向指定 Actor 发送消息
    pub async fn send_message(&self, actor_id: &ActorId, message: Vec<u8>) -> ActorResult<()> {
        let serial_number = actor_id.serial_number;
        match self.actors.get(&serial_number) {
            Some(container) => container.handle_message(message).await,
            None => Err(ActorError::ActorNotFound {
                actor_id: format!("{}", serial_number),
            }),
        }
    }

    /// 启动所有 Actor
    pub async fn start_all(&mut self) -> ActorResult<()> {
        // 首先初始化所有 Actor
        for (serial_number, container) in &mut self.actors {
            if let Err(e) = container.initialize().await {
                tracing::error!("Failed to initialize actor {}: {}", serial_number, e);
                return Err(e);
            }
        }

        // 然后启动所有 Actor
        for (serial_number, container) in &mut self.actors {
            if let Err(e) = container.start().await {
                tracing::error!("Failed to start actor {}: {}", serial_number, e);
                return Err(e);
            }
        }

        info!("All local actors started successfully");
        Ok(())
    }

    /// 停止所有 Actor
    pub async fn stop_all(&mut self) -> ActorResult<()> {
        for (serial_number, container) in &mut self.actors {
            if let Err(e) = container.stop().await {
                tracing::error!("Failed to stop actor {}: {}", serial_number, e);
            }
        }

        info!("All local actors stopped");
        Ok(())
    }

    /// 获取所有 Actor ID
    pub fn get_actor_ids(&self) -> Vec<ActorId> {
        self.actors
            .values()
            .map(|container| container.get_actor_id())
            .collect()
    }

    /// 检查 Actor 是否存在
    pub fn has_actor(&self, actor_id: &ActorId) -> bool {
        self.actors.contains_key(&actor_id.serial_number)
    }

    /// 检查 Actor 是否在运行中
    pub fn is_actor_running(&self, actor_id: &ActorId) -> bool {
        self.actors
            .get(&actor_id.serial_number)
            .map(|container| container.is_running())
            .unwrap_or(false)
    }
}

impl Default for LocalActorManager {
    fn default() -> Self {
        Self::new()
    }
}