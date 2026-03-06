//! Web Context - Web 环境专用的 Actor 上下文
//!
//! 与 actr_framework::Context 的区别：
//! - 不要求 Send + Sync（Service Worker 是单线程）
//! - async trait 使用 ?Send
//! - 适配浏览器环境的限制
//!
//! # 设计
//!
//! - `RuntimeBridge`: 运行时桥接 trait，解耦 context.rs 与 runtime.rs
//! - `WebContext`: 完整 Actor 上下文 trait（包含 RPC、发现、流 等所有能力）
//! - `RuntimeContext`（context.rs）: WebContext 的唯一实现，ServiceHandlerFn 直接使用此结构

use actr_protocol::{ActorResult, ActrId, ActrType, RpcRequest};
use bytes::Bytes;

/// RuntimeBridge - 运行时基础设施的抽象桥接
///
/// 提供 RuntimeContext 所需的底层能力（pending RPC 注册、发现、连接管理），
/// 避免 context.rs 直接依赖 runtime.rs（防止循环依赖）。
///
/// 实现方（SwRuntimeBridge）位于 runtime.rs，持有 SwRuntime、System 等引用。
#[async_trait::async_trait(?Send)]
pub trait RuntimeBridge {
    /// 注册 pending RPC（使 handle_fast_path 能识别响应）
    fn register_pending_rpc(&self, request_id: String);

    /// 发现目标 Actor（通过 Signaling 服务）
    async fn discover_target(&self, target_type: &ActrType) -> ActorResult<ActrId>;

    /// 确保与目标 Actor 的 WebRTC 连接已就绪并注册 ActrId → Dest 映射
    async fn ensure_connection(&self, target_id: &ActrId) -> ActorResult<()>;

    /// 注册流回调处理器
    fn register_stream_handler(
        &self,
        stream_id: String,
        callback: Box<dyn FnMut(Bytes) + 'static>,
    ) -> ActorResult<()>;

    /// 注销流回调处理器
    fn unregister_stream_handler(&self, stream_id: &str) -> ActorResult<()>;
}

/// Web 环境的 Actor 执行上下文
///
/// 对应 Native 的 Context trait，但适配单线程环境。
/// 包含所有 Actor 通信能力：RPC（typed + raw）、发现、流操作。
///
/// ServiceHandlerFn 直接使用 `Rc<RuntimeContext>` 而非 trait object，
/// 使 handler 可以调用所有方法（包含泛型方法 call<R>, tell<R>）。
#[async_trait::async_trait(?Send)]
pub trait WebContext {
    // ========== 基础信息 ==========

    /// 获取当前 Actor 的 ID
    fn self_id(&self) -> &ActrId;

    /// 获取调用方 Actor ID
    fn caller_id(&self) -> Option<&ActrId>;

    /// 获取分布式追踪 ID
    fn trace_id(&self) -> &str;

    /// 获取唯一请求 ID
    fn request_id(&self) -> &str;

    // ========== RPC 通信 ==========

    /// 发送原始 RPC 请求并等待响应（无类型安全）
    ///
    /// 适用于 UnifiedDispatcher 等需要动态 route_key 的场景。
    ///
    /// # 参数
    /// - `target`: 目标 Actor ID
    /// - `route_key`: 路由键（如 `"echo.EchoService.Echo"`）
    /// - `payload`: 序列化后的请求 payload
    /// - `timeout_ms`: 超时时间（毫秒）
    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: &[u8],
        timeout_ms: i64,
    ) -> ActorResult<Vec<u8>>;

    /// 发现目标 Actor（通过 Signaling 的服务发现 API）
    async fn discover(&self, target_type: &ActrType) -> ActorResult<ActrId>;

    // ========== 类型安全通信方法 ==========

    /// 发送类型安全的 RPC 请求并等待响应
    async fn call<R: RpcRequest>(&self, target: &ActrId, request: R) -> ActorResult<R::Response>;

    /// 发送单向消息（不等待响应）
    async fn tell<R: RpcRequest>(&self, target: &ActrId, request: R) -> ActorResult<()>;

    // ========== Stream 注册方法 ==========

    /// 注册流数据处理器
    ///
    /// 注册后，目标 Actor 发送的 STREAM_* 数据将通过 Fast Path 直接派发到 callback
    async fn register_stream(
        &self,
        stream_id: String,
        callback: Box<dyn FnMut(Bytes) + 'static>,
    ) -> ActorResult<()>;

    /// 注销流数据处理器
    async fn unregister_stream(&self, stream_id: &str) -> ActorResult<()>;

    /// 注册媒体轨道处理器
    ///
    /// 用于 WebRTC MediaTrack（音频/视频）的 Fast Path 处理
    async fn register_media_track(
        &self,
        track_id: String,
        callback: Box<dyn FnMut(Bytes) + 'static>,
    ) -> ActorResult<()>;

    /// 注销媒体轨道处理器
    async fn unregister_media_track(&self, track_id: &str) -> ActorResult<()>;

    // ========== Stream 发送方法 ==========

    /// 发送媒体采样数据（RTP 包）
    ///
    /// 通过 Fast Path 发送，延迟 < 1ms
    async fn send_media_sample(
        &self,
        target: &ActrId,
        track_id: &str,
        data: Bytes,
    ) -> ActorResult<()>;

    /// 发送流数据
    ///
    /// 通过 Fast Path 发送，延迟 ~3ms
    async fn send_data_stream(
        &self,
        target: &ActrId,
        stream_id: &str,
        data: Bytes,
    ) -> ActorResult<()>;
}
