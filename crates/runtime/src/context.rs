//! Runtime Context Implementation
//!
//! Implements the Context trait defined in actr-framework.

use crate::inbound::{DataStreamRegistry, MediaFrameRegistry};
use crate::outbound::OutGate;
use crate::wire::webrtc::SignalingClient;
#[cfg(feature = "opentelemetry")]
use crate::wire::webrtc::trace::inject_span_context_to_rpc;
use actr_framework::{Bytes, Context, DataStream, Dest, MediaSample};
use actr_protocol::{
    AIdCredential, ActorResult, ActrError, ActrId, ActrType, PayloadType, ProtocolError,
    RouteCandidatesRequest, RpcEnvelope, RpcRequest, route_candidates_request,
};
use async_trait::async_trait;
use futures_util::future::BoxFuture;
use std::sync::Arc;

/// RuntimeContext - Runtime's implementation of Context trait
///
/// # 设计特性
///
/// - **零虚函数**：内部使用 OutGate enum dispatch（非 dyn）
/// - **智能路由**：根据 Dest 自动选择 InprocOut 或 OutprocOut
/// - **完整实现**：包含 call/tell 的完整逻辑（编码、发送、解码）
/// - **类型安全**：泛型方法提供编译时类型检查
///
/// # 性能
///
/// - OutGate 是 enum，使用静态分发
/// - 编译器可完全内联整个调用链
/// - 零虚函数调用开销
#[derive(Clone)]
pub struct RuntimeContext {
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,
    inproc_gate: OutGate,                          // Shell/Local 调用 - 立即可用
    outproc_gate: Option<OutGate>,                 // 远程 Actor 调用 - 延迟初始化
    data_stream_registry: Arc<DataStreamRegistry>, // DataStream 回调注册表
    media_frame_registry: Arc<MediaFrameRegistry>, // MediaTrack 回调注册表
    signaling_client: Arc<dyn SignalingClient>,
    credential: AIdCredential,
}

impl RuntimeContext {
    /// 创建新的 RuntimeContext
    ///
    /// # 参数
    ///
    /// - `self_id`: 当前 Actor 的 ID
    /// - `caller_id`: 调用方 Actor ID（可选）
    /// - `request_id`: 当前请求唯一 ID
    /// - `inproc_gate`: 进程内通信 gate（立即可用）
    /// - `outproc_gate`: 跨进程通信 gate（可能为 None，等待 WebRTC 初始化）
    /// - `data_stream_registry`: DataStream 回调注册表
    /// - `media_frame_registry`: MediaTrack 回调注册表
    /// - `signaling_client`: 用于路由发现的信令客户端
    /// - `credential`: 该 Actor 的凭证（调用信令接口时使用）
    #[allow(clippy::too_many_arguments)] // Internal API - all parameters are required
    pub fn new(
        self_id: ActrId,
        caller_id: Option<ActrId>,
        request_id: String,
        inproc_gate: OutGate,
        outproc_gate: Option<OutGate>,
        data_stream_registry: Arc<DataStreamRegistry>,
        media_frame_registry: Arc<MediaFrameRegistry>,
        signaling_client: Arc<dyn SignalingClient>,
        credential: AIdCredential,
    ) -> Self {
        Self {
            self_id,
            caller_id,
            request_id,
            inproc_gate,
            outproc_gate,
            data_stream_registry,
            media_frame_registry,
            signaling_client,
            credential,
        }
    }

    /// 根据 Dest 选择合适的 gate
    ///
    /// - Dest::Shell → inproc_gate（立即可用）
    /// - Dest::Local → inproc_gate（立即可用）
    /// - Dest::Actor(_) → outproc_gate（需要检查是否已初始化）
    #[inline]
    fn select_gate(&self, dest: &Dest) -> ActorResult<&OutGate> {
        match dest {
            Dest::Shell | Dest::Local => Ok(&self.inproc_gate),
            Dest::Actor(_) => self.outproc_gate.as_ref().ok_or_else(|| {
                ProtocolError::Actr(ActrError::GateNotInitialized {
                    message: "OutprocOutGate not initialized yet (WebRTC setup in progress)"
                        .to_string(),
                })
            }),
        }
    }

    /// 从 Dest 提取目标 ActrId
    ///
    /// - Dest::Shell → self_id（Workload → App 反向调用）
    /// - Dest::Local → self_id（调用本地 Workload）
    /// - Dest::Actor(id) → id（远程调用）
    #[inline]
    fn extract_target_id<'a>(&'a self, dest: &'a Dest) -> &'a ActrId {
        match dest {
            Dest::Shell | Dest::Local => &self.self_id,
            Dest::Actor(id) => id,
        }
    }

    /// Execute a non-generic RPC request call (useful for language bindings).
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "RuntimeContext.call_raw")
    )]
    pub async fn call_raw(
        &self,
        target: &Dest,
        route_key: String,
        payload_type: PayloadType,
        payload: Bytes,
        timeout_ms: i64,
    ) -> ActorResult<Bytes> {
        #[cfg(feature = "opentelemetry")]
        use crate::wire::webrtc::trace::inject_span_context_to_rpc;

        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key,
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms,
        };
        #[cfg(feature = "opentelemetry")]
        inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);

        let gate = self.select_gate(target)?;
        let target_id = self.extract_target_id(target);
        gate.send_request_with_type(target_id, payload_type, envelope)
            .await
    }

    /// Execute a non-generic RPC message call (fire-and-forget, useful for language bindings).
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "RuntimeContext.tell_raw")
    )]
    pub async fn tell_raw(
        &self,
        target: &Dest,
        route_key: String,
        payload_type: PayloadType,
        payload: Bytes,
    ) -> ActorResult<()> {
        #[cfg(feature = "opentelemetry")]
        use crate::wire::webrtc::trace::inject_span_context_to_rpc;

        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key,
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms: 0,
        };
        #[cfg(feature = "opentelemetry")]
        inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);

        let gate = self.select_gate(target)?;
        let target_id = self.extract_target_id(target);
        gate.send_message_with_type(target_id, payload_type, envelope)
            .await
    }

    /// Send DataStream with an explicit payload type (lane selection).
    ///
    /// This is intended for language bindings; the `Context` trait method
    /// `send_data_stream()` currently defaults to StreamReliable.
    pub async fn send_data_stream_with_type(
        &self,
        target: &Dest,
        payload_type: actr_protocol::PayloadType,
        chunk: DataStream,
    ) -> ActorResult<()> {
        use actr_protocol::prost::Message as ProstMessage;

        let payload = chunk.encode_to_vec();

        let gate = self.select_gate(target)?;
        let target_id = self.extract_target_id(target);

        let result = gate
            .send_data_stream(target_id, payload_type, bytes::Bytes::from(payload).into())
            .await;

        result
    }
}

#[async_trait]
impl Context for RuntimeContext {
    // ========== 数据访问方法 ==========

    fn self_id(&self) -> &ActrId {
        &self.self_id
    }

    fn caller_id(&self) -> Option<&ActrId> {
        self.caller_id.as_ref()
    }

    fn request_id(&self) -> &str {
        &self.request_id
    }

    // ========== 通信能力方法 ==========
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "RuntimeContext.call")
    )]
    async fn call<R: RpcRequest>(&self, target: &Dest, request: R) -> ActorResult<R::Response> {
        use actr_protocol::prost::Message as ProstMessage;

        // 1. 编码请求为 protobuf bytes
        let payload: Bytes = request.encode_to_vec().into();

        // 2. 从 RpcRequest trait 获取 route_key（编译时确定）
        let route_key = R::route_key().to_string();

        // 3. 构造 RpcEnvelope（使用 W3C tracing）
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key,
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(), // 生成新的 request_id
            metadata: vec![],
            timeout_ms: 30000, // 默认 30 秒超时
        };
        // Inject tracing context from current span
        #[cfg(feature = "opentelemetry")]
        inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);

        // 4. 根据 Dest 选择 gate 并提取目标 ActrId（Shell/Local 立即可用，Actor 需要检查）
        let gate = self.select_gate(target)?;
        let target_id = self.extract_target_id(target);

        // 5. 通过 OutGate enum dispatch 发送（零虚函数调用！）
        // Respect request's declared payload type (lane selection)
        let response_bytes = gate
            .send_request_with_type(target_id, R::payload_type(), envelope)
            .await?;

        // 6. 解码响应（类型安全：R::Response）
        R::Response::decode(&*response_bytes).map_err(|e| {
            ProtocolError::Actr(ActrError::DecodeFailure {
                message: format!(
                    "Failed to decode {}: {}",
                    std::any::type_name::<R::Response>(),
                    e
                ),
            })
        })
    }

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "RuntimeContext.tell")
    )]
    async fn tell<R: RpcRequest>(&self, target: &Dest, message: R) -> ActorResult<()> {
        // 1. 编码消息
        let payload: Bytes = message.encode_to_vec().into();

        // 2. 获取 route_key
        let route_key = R::route_key().to_string();

        // 3. 构造 RpcEnvelope（fire-and-forget 语义）
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key,
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms: 0, // 0 表示不等待响应
        };
        // Inject tracing context from current span
        #[cfg(feature = "opentelemetry")]
        inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);

        // 4. 根据 Dest 选择 gate 并提取目标 ActrId（Shell/Local 立即可用，Actor 需要检查）
        let gate = self.select_gate(target)?;
        let target_id = self.extract_target_id(target);

        // 5. 通过 OutGate enum dispatch 发送（respect payload type）
        gate.send_message_with_type(target_id, R::payload_type(), envelope)
            .await
    }

    // ========== Fast Path: DataStream Methods ==========

    async fn register_stream<F>(&self, stream_id: String, callback: F) -> ActorResult<()>
    where
        F: Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        tracing::debug!(
            "📊 Registering DataStream callback for stream_id: {}",
            stream_id
        );
        self.data_stream_registry
            .register(stream_id, Arc::new(callback));
        Ok(())
    }

    async fn unregister_stream(&self, stream_id: &str) -> ActorResult<()> {
        tracing::debug!(
            "🚫 Unregistering DataStream callback for stream_id: {}",
            stream_id
        );
        self.data_stream_registry.unregister(stream_id);
        Ok(())
    }

    async fn send_data_stream(&self, target: &Dest, chunk: DataStream) -> ActorResult<()> {
        use actr_protocol::prost::Message as ProstMessage;

        // 1. Serialize DataStream to bytes
        let payload = chunk.encode_to_vec();

        tracing::debug!(
            "📤 Sending DataStream: stream_id={}, sequence={}, size={} bytes",
            chunk.stream_id,
            chunk.sequence,
            payload.len()
        );

        // 2. Select gate based on Dest
        let gate = self.select_gate(target)?;
        let target_id = self.extract_target_id(target);

        // 3. Send via OutGate with appropriate PayloadType
        // Use StreamReliable for reliable ordered transmission
        // TODO: Allow user to choose between StreamReliable and StreamLatencyFirst
        gate.send_data_stream(
            target_id,
            actr_protocol::PayloadType::StreamReliable,
            bytes::Bytes::from(payload),
        )
        .await
    }

    async fn discover_route_candidate(&self, target_type: &ActrType) -> ActorResult<ActrId> {
        if !self.signaling_client.is_connected() {
            return Err(ProtocolError::TransportError(
                "Signaling client is not connected.".to_string(),
            ));
        }

        let criteria = route_candidates_request::NodeSelectionCriteria {
            candidate_count: 1,
            ranking_factors: Vec::new(),
            minimal_dependency_requirement: None,
            minimal_health_requirement: None,
        };

        let request = RouteCandidatesRequest {
            target_type: target_type.clone(),
            criteria: Some(criteria),
            client_location: None,
            client_fingerprint: None,
        };

        let response = self
            .signaling_client
            .send_route_candidates_request(self.self_id.clone(), self.credential.clone(), request)
            .await
            .map_err(|e| {
                ProtocolError::TransportError(format!("Route candidates request failed: {e}"))
            })?;

        match response.result {
            Some(actr_protocol::route_candidates_response::Result::Success(ok)) => {
                ok.candidates.into_iter().next().ok_or_else(|| {
                    ProtocolError::TargetNotFound(format!(
                        "No route candidates for type {}.{}",
                        target_type.manufacturer, target_type.name
                    ))
                })
            }
            Some(actr_protocol::route_candidates_response::Result::Error(err)) => {
                Err(ProtocolError::TransportError(format!(
                    "Route candidates error {}: {}",
                    err.code, err.message
                )))
            }
            None => Err(ProtocolError::TransportError(
                "Route candidates response missing result".to_string(),
            )),
        }
    }

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "RuntimeContext.call_raw")
    )]
    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: Bytes,
    ) -> ActorResult<Bytes> {
        // 1. Construct RpcEnvelope with raw payload
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key: route_key.to_string(),
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms: 30000, // Default 30 second timeout
        };

        // Inject tracing context from current span
        #[cfg(feature = "opentelemetry")]
        inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);

        // 2. Select outproc gate (raw calls are always remote)
        let gate = self.outproc_gate.as_ref().ok_or_else(|| {
            ProtocolError::Actr(ActrError::GateNotInitialized {
                message: "OutprocOutGate not initialized yet (WebRTC setup in progress)"
                    .to_string(),
            })
        })?;

        // 3. Send request and return raw response bytes
        gate.send_request(target, envelope).await
    }

    // ========== Fast Path: MediaTrack Methods ==========

    async fn register_media_track<F>(&self, track_id: String, callback: F) -> ActorResult<()>
    where
        F: Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        tracing::debug!(
            "📹 Registering MediaTrack callback for track_id: {}",
            track_id
        );
        self.media_frame_registry
            .register(track_id, Arc::new(callback));
        Ok(())
    }

    async fn unregister_media_track(&self, track_id: &str) -> ActorResult<()> {
        tracing::debug!(
            "📹 Unregistering MediaTrack callback for track_id: {}",
            track_id
        );
        self.media_frame_registry.unregister(track_id);
        Ok(())
    }

    async fn send_media_sample(
        &self,
        target: &Dest,
        track_id: &str,
        sample: MediaSample,
    ) -> ActorResult<()> {
        // 1. Select appropriate gate based on Dest
        let gate = self.select_gate(target)?;

        // 2. Extract target ActrId
        let target_id = self.extract_target_id(target);

        // 3. Send via OutGate (delegates to WebRTC Track)
        gate.send_media_sample(target_id, track_id, sample).await
    }
}
