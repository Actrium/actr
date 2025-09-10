//! RemoteActor implementation - 远程 Actor 句柄实现
//!
//! RemoteActor 表示运行在其他进程或节点上的 Actor，只提供消息发送能力，不负责生命周期管理

use crate::context::Context;
use crate::error::{ActorError, ActorResult};
use shared_protocols::actor::ActorId;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// 远程 Actor 句柄 - Actor 的远程代理
#[derive(Debug, Clone)]
pub struct RemoteActor {
    /// 远程 Actor 的 ID
    actor_id: ActorId,
    /// 连接状态
    connection_state: Arc<tokio::sync::RwLock<ConnectionState>>,
    /// 上下文引用
    context: Arc<Context>,
    /// 连接元数据
    metadata: RemoteActorMetadata,
}

/// 连接状态
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConnectionState {
    /// 未连接
    Disconnected,
    /// 连接中
    Connecting,
    /// 已连接
    Connected { connected_at: Instant },
    /// 连接失败
    Failed {
        last_error: String,
        failed_at: Instant,
    },
}

/// 远程 Actor 元数据
#[derive(Debug, Clone)]
pub struct RemoteActorMetadata {
    /// Actor 类型名称
    pub type_name: String,
    /// 服务地址（可选）
    pub service_address: Option<String>,
    /// 最后活动时间
    pub last_activity: Arc<tokio::sync::RwLock<Option<Instant>>>,
    /// 连接超时配置
    pub connection_timeout: Duration,
    /// 消息超时配置
    pub message_timeout: Duration,
}

impl RemoteActor {
    /// 创建新的远程 Actor 句柄
    pub fn new(actor_id: ActorId, context: Arc<Context>, type_name: String) -> Self {
        Self {
            actor_id,
            connection_state: Arc::new(tokio::sync::RwLock::new(ConnectionState::Disconnected)),
            context,
            metadata: RemoteActorMetadata {
                type_name,
                service_address: None,
                last_activity: Arc::new(tokio::sync::RwLock::new(None)),
                connection_timeout: Duration::from_secs(10),
                message_timeout: Duration::from_secs(30),
            },
        }
    }

    /// 创建带有服务地址的远程 Actor 句柄
    pub fn with_service_address(
        actor_id: ActorId,
        context: Arc<Context>,
        type_name: String,
        service_address: String,
    ) -> Self {
        let mut remote_actor = Self::new(actor_id, context, type_name);
        remote_actor.metadata.service_address = Some(service_address);
        remote_actor
    }

    /// 获取 Actor ID
    pub fn get_actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// 获取 Actor 类型名称
    pub fn get_type_name(&self) -> &str {
        &self.metadata.type_name
    }

    /// 获取服务地址
    pub fn get_service_address(&self) -> Option<&str> {
        self.metadata.service_address.as_deref()
    }

    /// 建立连接
    pub async fn connect(&self) -> ActorResult<()> {
        {
            let mut state = self.connection_state.write().await;
            if let ConnectionState::Connected { .. } = *state {
                debug!(
                    "Already connected to actor: {}",
                    self.actor_id.serial_number
                );
                return Ok(());
            }
            *state = ConnectionState::Connecting;
        }

        info!(
            "Connecting to remote actor: {}",
            self.actor_id.serial_number
        );

        // 尝试建立连接
        match self.context.connect_to(&self.actor_id).await {
            Ok(()) => {
                let mut state = self.connection_state.write().await;
                *state = ConnectionState::Connected {
                    connected_at: Instant::now(),
                };

                // 更新最后活动时间
                *self.metadata.last_activity.write().await = Some(Instant::now());

                info!(
                    "Successfully connected to remote actor: {}",
                    self.actor_id.serial_number
                );
                Ok(())
            }
            Err(e) => {
                let mut state = self.connection_state.write().await;
                *state = ConnectionState::Failed {
                    last_error: e.to_string(),
                    failed_at: Instant::now(),
                };

                warn!(
                    "Failed to connect to remote actor {}: {}",
                    self.actor_id.serial_number, e
                );
                Err(e)
            }
        }
    }

    /// 断开连接
    pub async fn disconnect(&self) -> ActorResult<()> {
        info!(
            "Disconnecting from remote actor: {}",
            self.actor_id.serial_number
        );

        match self.context.disconnect_from(&self.actor_id).await {
            Ok(()) => {
                let mut state = self.connection_state.write().await;
                *state = ConnectionState::Disconnected;

                debug!(
                    "Successfully disconnected from remote actor: {}",
                    self.actor_id.serial_number
                );
                Ok(())
            }
            Err(e) => {
                warn!(
                    "Failed to disconnect from remote actor {}: {}",
                    self.actor_id.serial_number, e
                );
                Err(e)
            }
        }
    }

    /// 检查连接状态
    pub async fn is_connected(&self) -> bool {
        match &*self.connection_state.read().await {
            ConnectionState::Connected { .. } => {
                // 进一步检查连接是否仍然有效
                self.context.is_connected_to(&self.actor_id).await
            }
            _ => false,
        }
    }

    /// 发送单向消息 (tell)
    pub async fn tell<T>(&self, message: T) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        // 确保已连接
        if !self.is_connected().await {
            self.connect().await?;
        }

        debug!(
            "Sending tell message to remote actor: {}",
            self.actor_id.serial_number
        );

        match self.context.tell(&self.actor_id, message).await {
            Ok(()) => {
                // 更新最后活动时间
                *self.metadata.last_activity.write().await = Some(Instant::now());
                Ok(())
            }
            Err(e) => {
                warn!(
                    "Failed to send tell message to {}: {}",
                    self.actor_id.serial_number, e
                );
                // 连接可能已断开，更新状态
                let mut state = self.connection_state.write().await;
                *state = ConnectionState::Failed {
                    last_error: e.to_string(),
                    failed_at: Instant::now(),
                };
                Err(e)
            }
        }
    }

    /// 发送请求消息 (call)
    pub async fn call<Req, Resp>(&self, request: Req) -> ActorResult<Resp>
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        // 确保已连接
        if !self.is_connected().await {
            self.connect().await?;
        }

        debug!(
            "Sending call message to remote actor: {}",
            self.actor_id.serial_number
        );

        match self.context.call(&self.actor_id, request).await {
            Ok(response) => {
                // 更新最后活动时间
                *self.metadata.last_activity.write().await = Some(Instant::now());
                Ok(response)
            }
            Err(e) => {
                warn!(
                    "Failed to send call message to {}: {}",
                    self.actor_id.serial_number, e
                );
                // 连接可能已断开，更新状态
                let mut state = self.connection_state.write().await;
                *state = ConnectionState::Failed {
                    last_error: e.to_string(),
                    failed_at: Instant::now(),
                };
                Err(e)
            }
        }
    }

    /// 通过服务路由发送请求 (request)
    pub async fn request<Req, Resp>(&self, request: Req) -> ActorResult<Resp>
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        debug!(
            "Sending service request through remote actor: {}",
            self.actor_id.serial_number
        );

        match self.context.request(request).await {
            Ok(response) => {
                // 更新最后活动时间
                *self.metadata.last_activity.write().await = Some(Instant::now());
                Ok(response)
            }
            Err(e) => {
                warn!(
                    "Failed to send service request through {}: {}",
                    self.actor_id.serial_number, e
                );
                Err(e)
            }
        }
    }

    /// 通过服务路由发送通知 (notify)
    pub async fn notify<T>(&self, message: T) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        debug!(
            "Sending service notify through remote actor: {}",
            self.actor_id.serial_number
        );

        match self.context.notify(message).await {
            Ok(()) => {
                // 更新最后活动时间
                *self.metadata.last_activity.write().await = Some(Instant::now());
                Ok(())
            }
            Err(e) => {
                warn!(
                    "Failed to send service notify through {}: {}",
                    self.actor_id.serial_number, e
                );
                Err(e)
            }
        }
    }

    /// 获取连接状态
    pub(crate) async fn get_connection_state(&self) -> ConnectionState {
        self.connection_state.read().await.clone()
    }

    /// 获取最后活动时间
    pub async fn get_last_activity(&self) -> Option<Instant> {
        *self.metadata.last_activity.read().await
    }

    /// 设置连接超时时间
    pub fn set_connection_timeout(&mut self, timeout: Duration) {
        self.metadata.connection_timeout = timeout;
    }

    /// 设置消息超时时间
    pub fn set_message_timeout(&mut self, timeout: Duration) {
        self.metadata.message_timeout = timeout;
    }

    /// 检查连接是否超时
    pub async fn is_connection_timeout(&self) -> bool {
        if let Some(last_activity) = self.get_last_activity().await {
            last_activity.elapsed() > self.metadata.connection_timeout
        } else {
            true
        }
    }

    /// 获取连接统计信息
    pub async fn get_connection_stats(&self) -> RemoteActorStats {
        let state = self.get_connection_state().await;
        let last_activity = self.get_last_activity().await;

        let uptime = match &state {
            ConnectionState::Connected { connected_at } => Some(connected_at.elapsed()),
            _ => None,
        };

        RemoteActorStats {
            actor_id: self.actor_id.clone(),
            connection_state: state,
            last_activity,
            uptime,
        }
    }
}

/// 远程 Actor 统计信息
#[derive(Debug, Clone)]
pub struct RemoteActorStats {
    pub actor_id: ActorId,
    #[allow(dead_code)]
    pub(crate) connection_state: ConnectionState,
    pub last_activity: Option<Instant>,
    pub uptime: Option<Duration>,
}

/// 远程 Actor 管理器 - 管理多个远程 Actor 句柄
pub struct RemoteActorManager {
    /// 远程 Actor 句柄集合 (使用 serial_number 作为键)
    actors: std::collections::HashMap<u64, RemoteActor>,
    /// 上下文引用
    context: Arc<Context>,
}

impl RemoteActorManager {
    /// 创建新的远程 Actor 管理器
    pub fn new(context: Arc<Context>) -> Self {
        Self {
            actors: std::collections::HashMap::new(),
            context,
        }
    }

    /// 注册远程 Actor
    pub fn register_remote_actor(
        &mut self,
        actor_id: ActorId,
        type_name: String,
        service_address: Option<String>,
    ) -> ActorResult<()> {
        let serial_number = actor_id.serial_number;
        if self.actors.contains_key(&serial_number) {
            return Err(ActorError::InvalidState(format!(
                "Remote actor already registered: {}",
                serial_number
            )));
        }

        let remote_actor = if let Some(address) = service_address {
            RemoteActor::with_service_address(actor_id, self.context.clone(), type_name, address)
        } else {
            RemoteActor::new(actor_id, self.context.clone(), type_name)
        };

        self.actors.insert(serial_number, remote_actor);
        Ok(())
    }

    /// 获取远程 Actor 句柄
    pub fn get_remote_actor(&self, actor_id: &ActorId) -> Option<&RemoteActor> {
        self.actors.get(&actor_id.serial_number)
    }

    /// 获取远程 Actor 句柄（异步版本，用于 Arc<RwLock<RemoteActorManager>>）
    pub async fn get_remote_actor_async(
        manager: Arc<tokio::sync::RwLock<RemoteActorManager>>,
        actor_id: &ActorId,
    ) -> Option<RemoteActor> {
        let manager_guard = manager.read().await;
        manager_guard.actors.get(&actor_id.serial_number).cloned()
    }

    /// 获取可变的远程 Actor 句柄
    pub fn get_remote_actor_mut(&mut self, actor_id: &ActorId) -> Option<&mut RemoteActor> {
        self.actors.get_mut(&actor_id.serial_number)
    }

    /// 移除远程 Actor
    pub async fn remove_remote_actor(&mut self, actor_id: &ActorId) -> ActorResult<()> {
        let serial_number = actor_id.serial_number;
        if let Some(remote_actor) = self.actors.get(&serial_number) {
            // 先断开连接
            if let Err(e) = remote_actor.disconnect().await {
                warn!(
                    "Failed to disconnect from remote actor {} during removal: {}",
                    serial_number, e
                );
            }
        }

        self.actors.remove(&serial_number);
        Ok(())
    }

    /// 连接到所有远程 Actor
    pub async fn connect_all(&self) -> Vec<ActorResult<()>> {
        let mut results = Vec::new();

        for (serial_number, remote_actor) in &self.actors {
            debug!("Connecting to remote actor: {}", serial_number);
            let result = remote_actor.connect().await;
            results.push(result);
        }

        results
    }

    /// 断开所有远程 Actor 连接
    pub async fn disconnect_all(&self) -> Vec<ActorResult<()>> {
        let mut results = Vec::new();

        for (serial_number, remote_actor) in &self.actors {
            debug!("Disconnecting from remote actor: {}", serial_number);
            let result = remote_actor.disconnect().await;
            results.push(result);
        }

        results
    }

    /// 获取所有已连接的远程 Actor ID
    pub async fn get_connected_actors(&self) -> Vec<ActorId> {
        let mut connected = Vec::new();

        for (_, remote_actor) in &self.actors {
            if remote_actor.is_connected().await {
                connected.push(remote_actor.get_actor_id().clone());
            }
        }

        connected
    }

    /// 获取所有远程 Actor 统计信息
    pub async fn get_all_stats(&self) -> Vec<RemoteActorStats> {
        let mut stats = Vec::new();

        for remote_actor in self.actors.values() {
            stats.push(remote_actor.get_connection_stats().await);
        }

        stats
    }

    /// 清理超时的连接
    pub async fn cleanup_timeouts(&mut self) {
        let mut to_disconnect = Vec::new();

        for (serial_number, remote_actor) in &self.actors {
            if remote_actor.is_connection_timeout().await {
                to_disconnect.push(*serial_number);
            }
        }

        for serial_number in to_disconnect {
            if let Some(remote_actor) = self.actors.get(&serial_number) {
                warn!("Disconnecting timed out remote actor: {}", serial_number);
                if let Err(e) = remote_actor.disconnect().await {
                    warn!(
                        "Failed to disconnect timed out actor {}: {}",
                        serial_number, e
                    );
                }
            }
        }
    }

    /// 获取所有远程 Actor ID
    pub fn get_all_actor_ids(&self) -> Vec<ActorId> {
        self.actors
            .values()
            .map(|remote_actor| remote_actor.get_actor_id().clone())
            .collect()
    }

    /// 检查是否包含指定的远程 Actor
    pub fn contains_actor(&self, actor_id: &ActorId) -> bool {
        self.actors.contains_key(&actor_id.serial_number)
    }
}
