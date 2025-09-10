//! 消息处理和路由系统

use crate::context::Context;
use crate::error::{ActorResult, MessageError};
use async_trait::async_trait;
use prost::Message;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// 消息处理器 trait
///
/// 实现此 trait 为特定的消息类型提供处理逻辑。
/// 框架会自动根据消息类型路由到相应的处理器。
#[async_trait]
pub trait MessageHandler<T>: Send + Sync
where
    T: prost::Message + Send + Sync + 'static,
{
    /// 响应消息类型
    type Response: prost::Message + Send + Sync + 'static;

    /// 处理消息
    ///
    /// # 参数
    /// - `message`: 接收到的消息
    /// - `ctx`: Actor 上下文
    ///
    /// # 返回值
    /// 处理结果，可能包含响应消息
    async fn handle(&self, message: T, ctx: Arc<Context>) -> ActorResult<Self::Response>;
}

/// 流式消息处理器 trait
///
/// 用于处理需要低延迟的流式数据，会走快车道。
#[async_trait]
pub trait StreamMessageHandler<T>: Send + Sync
where
    T: prost::Message + Send + Sync + 'static,
{
    /// 处理流式消息
    ///
    /// 注意：流式消息处理器不返回响应，专注于低延迟处理。
    /// 如果需要发送响应，请在处理逻辑中通过 Context 主动发送。
    ///
    /// # 参数
    /// - `message`: 接收到的流式消息
    /// - `ctx`: Actor 上下文
    async fn handle_stream(&self, message: T, ctx: Arc<Context>) -> ActorResult<()>;
}

/// 消息包装器 - 用于类型擦除
#[allow(dead_code)]
pub(crate) struct MessageWrapper {
    pub type_name: String,
    pub payload: Vec<u8>,
    pub is_stream: bool,
}

/// 消息路由表
pub(crate) struct MessageRouter {
    /// 状态路径消息处理器（可靠处理）
    #[allow(dead_code)]
    state_handlers: HashMap<TypeId, Box<dyn MessageHandlerErased>>,
    /// 快车道消息处理器（流式处理）
    #[allow(dead_code)]
    stream_handlers: HashMap<TypeId, Box<dyn StreamMessageHandlerErased>>,
    /// 类型名称映射
    #[allow(dead_code)]
    type_names: HashMap<TypeId, String>,
}

impl MessageRouter {
    pub fn new() -> Self {
        Self {
            state_handlers: HashMap::new(),
            stream_handlers: HashMap::new(),
            type_names: HashMap::new(),
        }
    }

    /// 注册状态路径消息处理器
    #[allow(dead_code)]
    pub fn register_handler<T, H>(&mut self, handler: H)
    where
        T: prost::Message + Send + Sync + 'static,
        H: MessageHandler<T> + 'static,
    {
        let type_id = TypeId::of::<T>();
        let type_name = std::any::type_name::<T>().to_string();

        self.state_handlers
            .insert(type_id, Box::new(MessageHandlerWrapper::new(handler)));
        self.type_names.insert(type_id, type_name);
    }

    /// 注册快车道消息处理器
    #[allow(dead_code)]
    pub fn register_stream_handler<T, H>(&mut self, handler: H)
    where
        T: prost::Message + Send + Sync + 'static,
        H: StreamMessageHandler<T> + 'static,
    {
        let type_id = TypeId::of::<T>();
        let type_name = std::any::type_name::<T>().to_string();

        self.stream_handlers
            .insert(type_id, Box::new(StreamMessageHandlerWrapper::new(handler)));
        self.type_names.insert(type_id, type_name);
    }

    /// 路由并处理状态路径消息
    #[allow(dead_code)]
    pub async fn handle_state_message<T>(
        &self,
        message: T,
        ctx: Arc<Context>,
    ) -> ActorResult<Vec<u8>>
    where
        T: prost::Message + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();

        match self.state_handlers.get(&type_id) {
            Some(handler) => {
                let message_any = Box::new(message) as Box<dyn Any + Send>;
                handler.handle_any(message_any, ctx).await
            }
            None => {
                let type_name = self
                    .type_names
                    .get(&type_id)
                    .map(|s| s.as_str())
                    .unwrap_or("unknown");
                Err(MessageError::HandlerNotFound(type_name.to_string()).into())
            }
        }
    }

    /// 路由并处理快车道消息
    #[allow(dead_code)]
    pub async fn handle_stream_message<T>(&self, message: T, ctx: Arc<Context>) -> ActorResult<()>
    where
        T: prost::Message + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();

        match self.stream_handlers.get(&type_id) {
            Some(handler) => {
                let message_any = Box::new(message) as Box<dyn Any + Send>;
                handler.handle_stream_any(message_any, ctx).await
            }
            None => {
                let type_name = self
                    .type_names
                    .get(&type_id)
                    .map(|s| s.as_str())
                    .unwrap_or("unknown");
                Err(MessageError::HandlerNotFound(type_name.to_string()).into())
            }
        }
    }

    /// 根据原始字节数据路由消息
    pub async fn route_raw_message(
        &self,
        message_type: &str,
        _payload: &[u8],
        _is_stream: bool,
        _ctx: Arc<Context>,
    ) -> ActorResult<Option<Vec<u8>>> {
        // 这里需要根据 message_type 找到对应的类型并反序列化
        // 为简化演示，这里返回错误，实际实现需要一个类型注册系统
        Err(MessageError::HandlerNotFound(message_type.to_string()).into())
    }
}

/// 类型擦除的消息处理器 trait
#[async_trait]
trait MessageHandlerErased: Send + Sync {
    #[allow(dead_code)]
    async fn handle_any(
        &self,
        message: Box<dyn Any + Send>,
        ctx: Arc<Context>,
    ) -> ActorResult<Vec<u8>>;
}

/// 类型擦除的流式消息处理器 trait  
#[async_trait]
trait StreamMessageHandlerErased: Send + Sync {
    #[allow(dead_code)]
    async fn handle_stream_any(
        &self,
        message: Box<dyn Any + Send>,
        ctx: Arc<Context>,
    ) -> ActorResult<()>;
}

/// 消息处理器包装器
#[allow(dead_code)]
struct MessageHandlerWrapper<T, H>
where
    T: prost::Message + Send + Sync + 'static,
    H: MessageHandler<T>,
{
    handler: H,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, H> MessageHandlerWrapper<T, H>
where
    T: prost::Message + Send + Sync + 'static,
    H: MessageHandler<T>,
{
    #[allow(dead_code)]
    fn new(handler: H) -> Self {
        Self {
            handler,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, H> MessageHandlerErased for MessageHandlerWrapper<T, H>
where
    T: prost::Message + Send + Sync + 'static,
    H: MessageHandler<T>,
{
    async fn handle_any(
        &self,
        message: Box<dyn Any + Send>,
        ctx: Arc<Context>,
    ) -> ActorResult<Vec<u8>> {
        let message = *message
            .downcast::<T>()
            .map_err(|_| MessageError::DecodingFailed("Type cast failed".to_string()))?;

        let response = self.handler.handle(message, ctx).await?;
        Ok(response.encode_to_vec())
    }
}

/// 流式消息处理器包装器
#[allow(dead_code)]
struct StreamMessageHandlerWrapper<T, H>
where
    T: prost::Message + Send + Sync + 'static,
    H: StreamMessageHandler<T>,
{
    handler: H,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, H> StreamMessageHandlerWrapper<T, H>
where
    T: prost::Message + Send + Sync + 'static,
    H: StreamMessageHandler<T>,
{
    #[allow(dead_code)]
    fn new(handler: H) -> Self {
        Self {
            handler,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, H> StreamMessageHandlerErased for StreamMessageHandlerWrapper<T, H>
where
    T: prost::Message + Send + Sync + 'static,
    H: StreamMessageHandler<T>,
{
    async fn handle_stream_any(
        &self,
        message: Box<dyn Any + Send>,
        ctx: Arc<Context>,
    ) -> ActorResult<()> {
        let message = *message
            .downcast::<T>()
            .map_err(|_| MessageError::DecodingFailed("Type cast failed".to_string()))?;

        self.handler.handle_stream(message, ctx).await
    }
}

/// 消息优先级
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum MessagePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// 内部消息包装
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct InternalMessage {
    pub payload: Vec<u8>,
    pub message_type: String,
    pub priority: MessagePriority,
    pub is_stream: bool,
    pub trace_id: String,
    pub source_actor: Option<shared_protocols::actor::ActorId>,
    /// 创建时间戳 (Unix timestamp in nanoseconds)
    pub created_at: u64,
}

impl InternalMessage {
    #[allow(dead_code)]
    pub fn new<T>(
        message: T,
        priority: MessagePriority,
        is_stream: bool,
        trace_id: String,
        source_actor: Option<shared_protocols::actor::ActorId>,
    ) -> Self
    where
        T: prost::Message,
    {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
            
        Self {
            payload: message.encode_to_vec(),
            message_type: std::any::type_name::<T>().to_string(),
            priority,
            is_stream,
            trace_id,
            source_actor,
            created_at: timestamp,
        }
    }
}
