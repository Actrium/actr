//! ActrRef - Lightweight reference to a running Actor (Web 版本)
//!
//! # Design Philosophy
//!
//! `ActrRef` 是对运行中 Actor 的轻量级引用，提供：
//!
//! - **RPC calls**: 调用 Actor 方法（从 DOM 侧 → SW 侧）
//! - **Lifecycle control**: 关闭和等待完成
//!
//! # Key Characteristics
//!
//! - **Cloneable**: 可在多个任务间共享
//! - **Lightweight**: 只包含一个 Arc 到共享状态
//! - **Code-gen friendly**: RPC 方法将被代码生成并绑定到此类型
//!
//! # Usage
//!
//! ```rust,ignore
//! let actr = node.start().await?;
//!
//! // 克隆并在不同任务中使用
//! let actr1 = actr.clone();
//! wasm_bindgen_futures::spawn_local(async move {
//!     actr1.call(SomeRequest { ... }).await?;
//! });
//!
//! // 关闭
//! actr.shutdown();
//! actr.wait_for_shutdown().await;
//! ```

use std::marker::PhantomData;
use std::sync::Arc;

use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrError, ActrId, RpcEnvelope};
use bytes::Bytes;

use crate::outbound::InprocOutGate;
use crate::trace::inject_span_context_to_rpc;
use actr_framework::Workload;

/// ActrRef - Lightweight reference to a running Actor (Web 版本)
///
/// 这是 `ActrNode::start()` 返回的主要句柄
///
/// # Code Generation Pattern
///
/// `actr-cli` 代码生成器将为 `ActrRef` 生成类型安全的 RPC 方法
///
/// ## Proto Definition
///
/// ```protobuf
/// service EchoService {
///   rpc Echo(EchoRequest) returns (EchoResponse);
/// }
/// ```
///
/// ## Generated Code
///
/// ```rust,ignore
/// impl ActrRef<EchoServiceWorkload> {
///     pub async fn echo(&self, request: EchoRequest) -> ActorResult<EchoResponse> {
///         self.call(request).await
///     }
/// }
/// ```
pub struct ActrRef<W: Workload> {
    pub(crate) shared: Arc<ActrRefShared>,
    _phantom: PhantomData<W>,
}

impl<W: Workload> Clone for ActrRef<W> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            _phantom: PhantomData,
        }
    }
}

/// Shared state between all ActrRef clones
///
/// 这是内部实现细节。当最后一个 `ActrRef` 被 drop 时，
/// 此结构的 `Drop` impl 将触发关闭并清理所有资源。
pub(crate) struct ActrRefShared {
    /// Actor ID
    pub(crate) actor_id: ActrId,

    /// Inproc gate for DOM → SW RPC
    /// (注意：与 actr 不同，Web 版本只需要 InprocOut)
    pub(crate) inproc_gate: Arc<InprocOutGate>,

    /// Shutdown flag
    pub(crate) shutdown: Arc<parking_lot::Mutex<bool>>,
}

impl<W: Workload> ActrRef<W> {
    /// Create new ActrRef from shared state
    ///
    /// 这是内部 API，由 `ActrNode::start()` 使用
    pub(crate) fn new(shared: Arc<ActrRefShared>) -> Self {
        Self {
            shared,
            _phantom: PhantomData,
        }
    }

    /// Get Actor ID
    pub fn actor_id(&self) -> &ActrId {
        &self.shared.actor_id
    }

    /// Call Actor method (DOM → SW RPC)
    ///
    /// 这是一个通用方法，由代码生成的 RPC 方法使用。
    /// 大多数用户应该使用生成的方法。
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // 通用调用
    /// let response: EchoResponse = actr.call(EchoRequest {
    ///     message: "Hello".to_string(),
    /// }).await?;
    ///
    /// // 生成的方法（推荐）
    /// let response = actr.echo(EchoRequest {
    ///     message: "Hello".to_string(),
    /// }).await?;
    /// ```
    pub async fn call<R>(&self, request: R) -> ActorResult<R::Response>
    where
        R: actr_protocol::RpcRequest + ProstMessage,
    {
        // 编码请求
        let payload: Bytes = request.encode_to_vec().into();

        // 创建 envelope
        let mut envelope = RpcEnvelope {
            route_key: R::route_key().to_string(),
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: format!("req-{}", js_sys::Math::random()),
            metadata: vec![],
            timeout_ms: 30000,
        };

        // Inject trace context to RPC envelope
        inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);

        // 发送请求并等待响应
        let response_bytes = self
            .shared
            .inproc_gate
            .send_request(&self.shared.actor_id, envelope)
            .await?;

        // 解码响应
        R::Response::decode(&*response_bytes)
            .map_err(|e| ActrError::DecodeFailure(format!("Failed to decode response: {e}")))
    }

    /// Send one-way message to Actor (DOM → SW, fire-and-forget)
    ///
    /// 与 `call()` 不同，此方法不等待响应。
    /// 用于不需要确认的通知或命令。
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // 发送通知而不等待响应
    /// actr.tell(LogEvent {
    ///     level: "INFO".to_string(),
    ///     message: "User logged in".to_string(),
    /// }).await?;
    /// ```
    pub async fn tell<R>(&self, message: R) -> ActorResult<()>
    where
        R: actr_protocol::RpcRequest + ProstMessage,
    {
        // 编码消息
        let payload: Bytes = message.encode_to_vec().into();

        // Create envelope with initial traceparent and tracestate set to None
        let mut envelope = RpcEnvelope {
            route_key: R::route_key().to_string(),
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: format!("req-{}", js_sys::Math::random()),
            metadata: vec![],
            timeout_ms: 0, // 单向消息无超时
        };

        // Inject trace context to RPC envelope
        inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);

        // 发送消息不等待响应
        self.shared
            .inproc_gate
            .send_message(&self.shared.actor_id, envelope)
            .await
    }

    /// Trigger Actor shutdown
    ///
    /// 这会通知 Actor 停止，但不等待完成。
    /// 使用 `wait_for_shutdown()` 等待清理完成。
    pub fn shutdown(&self) {
        log::info!("🛑 Shutdown requested for Actor {:?}", self.shared.actor_id);
        let mut shutdown = self.shared.shutdown.lock();
        *shutdown = true;
    }

    /// Wait for Actor to fully shutdown
    ///
    /// 这会等待 shutdown 信号被触发。
    /// Web 版本使用轮询实现（因为没有 tokio）。
    pub async fn wait_for_shutdown(&self) {
        loop {
            let is_shutdown = *self.shared.shutdown.lock();
            if is_shutdown {
                break;
            }

            // 等待一小段时间（使用 gloo_timers，兼容 Service Worker 环境）
            gloo_timers::future::TimeoutFuture::new(100).await;
        }
    }

    /// Check if Actor is shutting down
    pub fn is_shutting_down(&self) -> bool {
        *self.shared.shutdown.lock()
    }
}

impl Drop for ActrRefShared {
    fn drop(&mut self) {
        log::info!(
            "🧹 ActrRefShared dropping - cleaning up Actor {:?}",
            self.actor_id
        );

        // 设置 shutdown flag
        *self.shutdown.lock() = true;

        log::debug!("✅ Actor {:?} marked for shutdown", self.actor_id);
    }
}
