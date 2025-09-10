//! Actor 上下文实现
//!
//! Context 是 Actor 与系统交互的统一接口，遵循 "请求而非命令" 的设计原则。

use crate::error::{ActorError, ActorResult};
use shared_protocols::actor::ActorId;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Actor 上下文 - Actor 与外部世界交互的唯一接口
#[derive(Clone)]
pub struct Context {
    /// 当前 Actor 的 ID
    pub actor_id: ActorId,
    /// 调用者 ID（在处理来自其他 Actor 的消息时设置）
    pub caller_id: Option<ActorId>,
    /// 分布式追踪 ID
    pub trace_id: String,
    /// 系统句柄
    system_handle: Arc<ActorSystemHandle>,
    /// Fast Path 回调注册表
    fastpath_registry: Arc<tokio::sync::RwLock<HashMap<String, Box<dyn FastPathCallback>>>>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("actor_id", &self.actor_id)
            .field("caller_id", &self.caller_id)
            .field("trace_id", &self.trace_id)
            .field("system_handle", &"<ActorSystemHandle>")
            .field(
                "fastpath_registry",
                &format!(
                    "HashMap<String, Box<dyn FastPathCallback>> with {} entries",
                    self.fastpath_registry
                        .try_read()
                        .map(|r| r.len())
                        .unwrap_or(0)
                ),
            )
            .finish()
    }
}

impl Context {
    /// 创建新的上下文
    pub fn new(
        actor_id: ActorId,
        caller_id: Option<ActorId>,
        system_handle: Arc<ActorSystemHandle>,
    ) -> Self {
        Self {
            actor_id,
            caller_id,
            trace_id: Uuid::new_v4().to_string(),
            system_handle,
            fastpath_registry: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// 创建带有特定 trace_id 的上下文
    pub fn with_trace_id(
        actor_id: ActorId,
        caller_id: Option<ActorId>,
        trace_id: String,
        system_handle: Arc<ActorSystemHandle>,
    ) -> Self {
        Self {
            actor_id,
            caller_id,
            trace_id,
            system_handle,
            fastpath_registry: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// 发送单向消息给其他 Actor
    ///
    /// # 参数
    /// - `target`: 目标 Actor ID
    /// - `message`: 消息内容
    pub async fn tell<T>(&self, target: &ActorId, message: T) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        self.system_handle
            .send_message(target, message, &self.trace_id)
            .await
    }

    /// 调用其他 Actor 并等待响应
    ///
    /// # 参数
    /// - `target`: 目标 Actor ID
    /// - `request`: 请求消息
    ///
    /// # 返回值
    /// 响应消息
    pub async fn call<Req, Resp>(&self, target: &ActorId, request: Req) -> ActorResult<Resp>
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        self.system_handle
            .call_actor(target, request, &self.trace_id)
            .await
    }

    /// 通过服务路由发送请求并等待响应
    ///
    /// # 参数
    /// - `request`: 请求消息
    ///
    /// # 返回值
    /// 响应消息
    ///
    /// # 说明
    /// 此方法使用 actr.toml 中的路由配置来确定目标服务
    pub async fn request<Req, Resp>(&self, request: Req) -> ActorResult<Resp>
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        self.system_handle
            .route_request(request, &self.trace_id)
            .await
    }

    /// 通过服务路由发送通知消息
    ///
    /// # 参数
    /// - `message`: 通知消息
    ///
    /// # 说明
    /// 此方法使用 actr.toml 中的路由配置来确定目标服务
    pub async fn notify<T>(&self, message: T) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        self.system_handle
            .route_notify(message, &self.trace_id)
            .await
    }

    /// 延迟发送消息
    ///
    /// # 参数
    /// - `target`: 目标 Actor ID
    /// - `message`: 消息内容
    /// - `delay`: 延迟时间
    pub async fn schedule_tell<T>(
        &self,
        target: &ActorId,
        message: T,
        delay: std::time::Duration,
    ) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        self.system_handle
            .schedule_message(target, message, delay, &self.trace_id)
            .await
    }

    /// 获取当前连接的对等 Actor 列表
    pub async fn get_connected_peers(&self) -> Vec<ActorId> {
        self.system_handle.get_connected_peers().await
    }

    /// 检查与特定 Actor 的连接状态
    pub async fn is_connected_to(&self, actor_id: &ActorId) -> bool {
        self.system_handle.is_connected_to(actor_id).await
    }

    /// 主动连接到另一个 Actor
    pub async fn connect_to(&self, actor_id: &ActorId) -> ActorResult<()> {
        self.system_handle.connect_to_actor(actor_id).await
    }

    /// 断开与特定 Actor 的连接
    pub async fn disconnect_from(&self, actor_id: &ActorId) -> ActorResult<()> {
        self.system_handle.disconnect_from_actor(actor_id).await
    }

    // 日志方法
    pub fn log_debug(&self, message: &str) {
        debug!(
            actor_id = %self.actor_id.serial_number,
            trace_id = %self.trace_id,
            caller = ?self.caller_id.as_ref().map(|id| id.serial_number),
            "{}",
            message
        );
    }

    pub fn log_info(&self, message: &str) {
        info!(
            actor_id = %self.actor_id.serial_number,
            trace_id = %self.trace_id,
            caller = ?self.caller_id.as_ref().map(|id| id.serial_number),
            "{}",
            message
        );
    }

    pub fn log_warn(&self, message: &str) {
        warn!(
            actor_id = %self.actor_id.serial_number,
            trace_id = %self.trace_id,
            caller = ?self.caller_id.as_ref().map(|id| id.serial_number),
            "{}",
            message
        );
    }

    pub fn log_error(&self, message: &str) {
        error!(
            actor_id = %self.actor_id.serial_number,
            trace_id = %self.trace_id,
            caller = ?self.caller_id.as_ref().map(|id| id.serial_number),
            "{}",
            message
        );
    }

    /// 获取调用者信息（如果有）
    pub fn get_caller_id(&self) -> Option<&ActorId> {
        self.caller_id.as_ref()
    }

    /// 获取当前 Actor ID
    pub fn get_actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// 获取追踪 ID
    pub fn get_trace_id(&self) -> &str {
        &self.trace_id
    }

    /// 创建并发句柄（用于 Fast Path 操作）
    pub fn create_concurrent_handle(&self, stream_id: String) -> ConcurrentHandle {
        ConcurrentHandle {
            actor_id: self.actor_id.clone(),
            stream_id,
            context: Arc::new(self.clone()),
        }
    }

    /// 注册 Fast Path 回调
    pub async fn register_fastpath_callback<F>(
        &self,
        stream_id: String,
        callback: F,
    ) -> ActorResult<()>
    where
        F: Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let mut registry = self.fastpath_registry.write().await;
        registry.insert(stream_id, Box::new(callback));
        Ok(())
    }

    /// 注销 Fast Path 回调
    pub async fn unregister_fastpath_callback(&self, stream_id: &str) -> ActorResult<()> {
        let mut registry = self.fastpath_registry.write().await;
        registry.remove(stream_id);
        Ok(())
    }

    /// 调用 Fast Path 回调（由系统内部使用）
    pub async fn invoke_fastpath_callback(
        &self,
        stream_id: &str,
        data: Vec<u8>,
    ) -> ActorResult<()> {
        let registry = self.fastpath_registry.read().await;
        if let Some(callback) = registry.get(stream_id) {
            callback.call(data).await;
            Ok(())
        } else {
            Err(ActorError::ContextError(format!(
                "Fast path callback not found for stream: {}",
                stream_id
            )))
        }
    }
}

/// Fast Path 回调 trait
#[async_trait::async_trait]
pub trait FastPathCallback: Send + Sync {
    async fn call(&self, data: Vec<u8>);
}

#[async_trait::async_trait]
impl<F> FastPathCallback for F
where
    F: Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
{
    async fn call(&self, data: Vec<u8>) {
        self(data).await;
    }
}

/// 并发句柄 - Actor 在 Fast Path 中的线程安全代理
#[derive(Clone, Debug)]
pub struct ConcurrentHandle {
    /// 关联的 Actor ID
    pub actor_id: ActorId,
    /// 流 ID
    pub stream_id: String,
    /// 上下文引用
    context: Arc<Context>,
}

impl ConcurrentHandle {
    /// 处理数据块（示例方法）
    pub async fn handle_data_chunk(&self, chunk: Vec<u8>) {
        // 执行可以在当前线程完成的、无状态的纯计算
        let _processed_data = self.process_chunk(chunk);

        // 实际使用中，这里会创建具体的消息类型并发送到状态路径
        // 目前作为示例，我们记录处理信息
        self.log_info("Data chunk processed in Fast Path");
    }

    /// 处理数据块的纯计算逻辑
    fn process_chunk(&self, chunk: Vec<u8>) -> Vec<u8> {
        // 这里可以执行任何不涉及状态修改的计算
        // 例如：数据转换、验证、压缩等
        chunk
    }

    /// 创建处理结果消息（需要根据具体业务逻辑实现）
    #[allow(dead_code)]
    fn create_processed_message(&self, _processed_data: Vec<u8>) -> ActorResult<Vec<u8>> {
        // 这里应该创建具体的消息类型
        // 由于我们还没有定义具体的消息类型，这里返回错误
        Err(ActorError::ContextError(
            "Message creation not implemented".to_string(),
        ))
    }

    /// 获取流 ID
    pub fn get_stream_id(&self) -> &str {
        &self.stream_id
    }

    /// 获取关联的 Actor ID
    pub fn get_actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// 记录日志（使用上下文的日志功能）
    pub fn log_info(&self, message: &str) {
        self.context
            .log_info(&format!("Stream {}: {}", self.stream_id, message));
    }

    pub fn log_error(&self, message: &str) {
        self.context
            .log_error(&format!("Stream {}: {}", self.stream_id, message));
    }
}

/// Actor 系统句柄 - 内部系统功能的接口
pub struct ActorSystemHandle {
    /// 系统命令发送通道
    pub(crate) command_tx: mpsc::UnboundedSender<SystemCommand>,
    /// 对等连接状态
    pub(crate) peer_connections: Arc<tokio::sync::RwLock<HashMap<String, PeerConnectionInfo>>>,
    /// 待处理的调用请求
    pub(crate) pending_calls: Arc<tokio::sync::RwLock<HashMap<String, PendingCall>>>,
    /// Fast Path 回调注册表
    pub(crate) fastpath_callbacks:
        Arc<tokio::sync::RwLock<HashMap<String, Box<dyn FastPathCallback>>>>,
}

impl std::fmt::Debug for ActorSystemHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorSystemHandle")
            .field("command_tx", &"<mpsc::UnboundedSender<SystemCommand>>")
            .field(
                "peer_connections",
                &format!(
                    "RwLock<HashMap> with {} entries",
                    self.peer_connections
                        .try_read()
                        .map(|r| r.len())
                        .unwrap_or(0)
                ),
            )
            .field(
                "pending_calls",
                &format!(
                    "RwLock<HashMap> with {} entries",
                    self.pending_calls.try_read().map(|r| r.len()).unwrap_or(0)
                ),
            )
            .field(
                "fastpath_callbacks",
                &format!(
                    "RwLock<HashMap> with {} entries",
                    self.fastpath_callbacks
                        .try_read()
                        .map(|r| r.len())
                        .unwrap_or(0)
                ),
            )
            .finish()
    }
}

impl ActorSystemHandle {
    pub(crate) fn new(command_tx: mpsc::UnboundedSender<SystemCommand>) -> Self {
        Self {
            command_tx,
            peer_connections: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            pending_calls: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            fastpath_callbacks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// 创建占位符句柄（用于测试和初始化）
    pub fn placeholder() -> Arc<Self> {
        let (tx, _rx) = mpsc::unbounded_channel();
        Arc::new(Self::new(tx))
    }

    async fn send_message<T>(&self, target: &ActorId, message: T, trace_id: &str) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        let command = SystemCommand::SendMessage {
            target: target.clone(),
            payload: message.encode_to_vec(),
            trace_id: trace_id.to_string(),
        };

        self.command_tx
            .send(command)
            .map_err(|_| ActorError::ContextError("System command channel closed".to_string()))?;

        Ok(())
    }

    async fn call_actor<Req, Resp>(
        &self,
        target: &ActorId,
        request: Req,
        trace_id: &str,
    ) -> ActorResult<Resp>
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        let call_id = Uuid::new_v4().to_string();
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        // 注册待处理的调用
        {
            let mut pending_calls = self.pending_calls.write().await;
            pending_calls.insert(
                call_id.clone(),
                PendingCall {
                    response_tx,
                    created_at: std::time::Instant::now(),
                },
            );
        }

        let command = SystemCommand::CallActor {
            target: target.clone(),
            payload: request.encode_to_vec(),
            call_id: call_id.clone(),
            trace_id: trace_id.to_string(),
        };

        self.command_tx
            .send(command)
            .map_err(|_| ActorError::ContextError("System command channel closed".to_string()))?;

        // 等待响应（带超时）
        match tokio::time::timeout(std::time::Duration::from_secs(30), response_rx).await {
            Ok(Ok(response_data)) => {
                let response = Resp::decode(response_data.as_slice()).unwrap_or_default();
                Ok(response)
            }
            Ok(Err(_)) => Err(ActorError::ContextError(
                "Call response channel closed".to_string(),
            )),
            Err(_) => {
                // 超时，清理待处理的调用
                self.pending_calls.write().await.remove(&call_id);
                Err(ActorError::Timeout("Actor call timed out".to_string()))
            }
        }
    }

    async fn schedule_message<T>(
        &self,
        target: &ActorId,
        message: T,
        delay: std::time::Duration,
        trace_id: &str,
    ) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        let command = SystemCommand::ScheduleMessage {
            target: target.clone(),
            payload: message.encode_to_vec(),
            delay,
            trace_id: trace_id.to_string(),
        };

        self.command_tx
            .send(command)
            .map_err(|_| ActorError::ContextError("System command channel closed".to_string()))?;

        Ok(())
    }

    async fn route_request<Req, Resp>(&self, request: Req, trace_id: &str) -> ActorResult<Resp>
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        let command = SystemCommand::RouteRequest {
            payload: request.encode_to_vec(),
            message_type: std::any::type_name::<Req>().to_string(),
            trace_id: trace_id.to_string(),
        };

        let call_id = Uuid::new_v4().to_string();
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        // 注册待处理的调用
        {
            let mut pending_calls = self.pending_calls.write().await;
            pending_calls.insert(
                call_id.clone(),
                PendingCall {
                    response_tx,
                    created_at: std::time::Instant::now(),
                },
            );
        }

        self.command_tx
            .send(command)
            .map_err(|_| ActorError::ContextError("System command channel closed".to_string()))?;

        // 等待响应（带超时）
        match tokio::time::timeout(std::time::Duration::from_secs(30), response_rx).await {
            Ok(Ok(response_data)) => {
                let response = Resp::decode(response_data.as_slice()).unwrap_or_default();
                Ok(response)
            }
            Ok(Err(_)) => Err(ActorError::ContextError(
                "Route request response channel closed".to_string(),
            )),
            Err(_) => {
                // 超时，清理待处理的调用
                self.pending_calls.write().await.remove(&call_id);
                Err(ActorError::Timeout("Route request timed out".to_string()))
            }
        }
    }

    async fn route_notify<T>(&self, message: T, trace_id: &str) -> ActorResult<()>
    where
        T: prost::Message + Send + 'static,
    {
        let command = SystemCommand::RouteNotify {
            payload: message.encode_to_vec(),
            message_type: std::any::type_name::<T>().to_string(),
            trace_id: trace_id.to_string(),
        };

        self.command_tx
            .send(command)
            .map_err(|_| ActorError::ContextError("System command channel closed".to_string()))?;

        Ok(())
    }

    async fn get_connected_peers(&self) -> Vec<ActorId> {
        let connections = self.peer_connections.read().await;
        connections
            .values()
            .map(|info| info.actor_id.clone())
            .collect()
    }

    async fn is_connected_to(&self, actor_id: &ActorId) -> bool {
        let connections = self.peer_connections.read().await;
        let key = format!(
            "{}_{}",
            actor_id.serial_number,
            actor_id
                .r#type
                .as_ref()
                .map(|t| t.name.as_str())
                .unwrap_or("unknown")
        );
        connections.contains_key(&key)
    }

    async fn connect_to_actor(&self, actor_id: &ActorId) -> ActorResult<()> {
        let command = SystemCommand::ConnectToPeer {
            target: actor_id.clone(),
        };

        self.command_tx
            .send(command)
            .map_err(|_| ActorError::ContextError("System command channel closed".to_string()))?;

        Ok(())
    }

    async fn disconnect_from_actor(&self, actor_id: &ActorId) -> ActorResult<()> {
        let command = SystemCommand::DisconnectFromPeer {
            target: actor_id.clone(),
        };

        self.command_tx
            .send(command)
            .map_err(|_| ActorError::ContextError("System command channel closed".to_string()))?;

        Ok(())
    }
}

/// 系统内部命令
pub(crate) enum SystemCommand {
    SendMessage {
        target: ActorId,
        payload: Vec<u8>,
        trace_id: String,
    },
    CallActor {
        target: ActorId,
        payload: Vec<u8>,
        call_id: String,
        trace_id: String,
    },
    ScheduleMessage {
        target: ActorId,
        payload: Vec<u8>,
        delay: std::time::Duration,
        trace_id: String,
    },
    ConnectToPeer {
        target: ActorId,
    },
    DisconnectFromPeer {
        target: ActorId,
    },
    /// 服务路由请求命令
    RouteRequest {
        payload: Vec<u8>,
        message_type: String,
        trace_id: String,
    },
    /// 服务路由通知命令
    RouteNotify {
        payload: Vec<u8>,
        message_type: String,
        trace_id: String,
    },
    /// Fast Path 操作命令
    #[allow(dead_code)]
    RegisterFastPathCallback {
        stream_id: String,
        callback: Box<dyn FastPathCallback>,
    },
    #[allow(dead_code)]
    UnregisterFastPathCallback {
        stream_id: String,
    },
    #[allow(dead_code)]
    InvokeFastPathCallback {
        stream_id: String,
        data: Vec<u8>,
    },
    #[allow(dead_code)]
    Shutdown,
}

impl std::fmt::Debug for SystemCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemCommand::SendMessage {
                target,
                payload,
                trace_id,
            } => f
                .debug_struct("SendMessage")
                .field("target", target)
                .field("payload_len", &payload.len())
                .field("trace_id", trace_id)
                .finish(),
            SystemCommand::CallActor {
                target,
                payload,
                call_id,
                trace_id,
            } => f
                .debug_struct("CallActor")
                .field("target", target)
                .field("payload_len", &payload.len())
                .field("call_id", call_id)
                .field("trace_id", trace_id)
                .finish(),
            SystemCommand::ScheduleMessage {
                target,
                payload,
                delay,
                trace_id,
            } => f
                .debug_struct("ScheduleMessage")
                .field("target", target)
                .field("payload_len", &payload.len())
                .field("delay", delay)
                .field("trace_id", trace_id)
                .finish(),
            SystemCommand::ConnectToPeer { target } => f
                .debug_struct("ConnectToPeer")
                .field("target", target)
                .finish(),
            SystemCommand::DisconnectFromPeer { target } => f
                .debug_struct("DisconnectFromPeer")
                .field("target", target)
                .finish(),
            SystemCommand::RouteRequest {
                payload,
                message_type,
                trace_id,
            } => f
                .debug_struct("RouteRequest")
                .field("payload_len", &payload.len())
                .field("message_type", message_type)
                .field("trace_id", trace_id)
                .finish(),
            SystemCommand::RouteNotify {
                payload,
                message_type,
                trace_id,
            } => f
                .debug_struct("RouteNotify")
                .field("payload_len", &payload.len())
                .field("message_type", message_type)
                .field("trace_id", trace_id)
                .finish(),
            SystemCommand::RegisterFastPathCallback {
                stream_id,
                callback: _,
            } => f
                .debug_struct("RegisterFastPathCallback")
                .field("stream_id", stream_id)
                .field("callback", &"<FastPathCallback>")
                .finish(),
            SystemCommand::UnregisterFastPathCallback { stream_id } => f
                .debug_struct("UnregisterFastPathCallback")
                .field("stream_id", stream_id)
                .finish(),
            SystemCommand::InvokeFastPathCallback { stream_id, data } => f
                .debug_struct("InvokeFastPathCallback")
                .field("stream_id", stream_id)
                .field("data_len", &data.len())
                .finish(),
            SystemCommand::Shutdown => f.write_str("Shutdown"),
        }
    }
}

/// 对等连接信息
#[derive(Debug, Clone)]
pub(crate) struct PeerConnectionInfo {
    pub actor_id: ActorId,
    #[allow(dead_code)]
    pub connected_at: std::time::Instant,
    #[allow(dead_code)]
    pub last_activity: std::time::Instant,
}

/// 待处理的调用请求
#[derive(Debug)]
pub(crate) struct PendingCall {
    #[allow(dead_code)]
    pub response_tx: tokio::sync::oneshot::Sender<Vec<u8>>,
    #[allow(dead_code)]
    pub created_at: std::time::Instant,
}
