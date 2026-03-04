//! Context - Actor 内部执行上下文
//!
//! 对标 actr 的 Context trait，提供 Actor 内部的通信能力

use std::rc::Rc;

use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrId, ActrType, RpcEnvelope, RpcRequest};
use bytes::Bytes;

use crate::WebContext;
use crate::outbound::OutGate;
use crate::web_context::RuntimeBridge;

/// RuntimeContext - Actor 运行时上下文（Web 实现）
///
/// 对标 actr 的 RuntimeContext
pub struct RuntimeContext {
    /// 当前 Actor ID
    self_id: ActrId,

    /// 调用方 Actor ID
    caller_id: Option<ActrId>,

    /// 追踪 ID
    traceparent: String,
    tracestate: String,

    /// 请求 ID
    request_id: String,

    /// 出站 gate
    outproc_gate: OutGate,

    /// 运行时桥接（用于 call_raw、discover 等需要底层 runtime 支持的操作）
    bridge: Option<Rc<dyn RuntimeBridge>>,
}

impl RuntimeContext {
    /// 创建新的 Context
    pub fn new(
        self_id: ActrId,
        caller_id: Option<ActrId>,
        traceparent: String,
        tracestate: String,
        request_id: String,
        outproc_gate: OutGate,
    ) -> Self {
        Self {
            self_id,
            caller_id,
            traceparent,
            tracestate,
            request_id,
            outproc_gate,
            bridge: None,
        }
    }

    /// 创建带有 RuntimeBridge 的 Context（用于 handler 上下文）
    pub fn with_bridge(mut self, bridge: Rc<dyn RuntimeBridge>) -> Self {
        self.bridge = Some(bridge);
        self
    }
}

#[async_trait::async_trait(?Send)]
impl WebContext for RuntimeContext {
    // ========== 基础信息 ==========

    fn self_id(&self) -> &ActrId {
        &self.self_id
    }

    fn caller_id(&self) -> Option<&ActrId> {
        self.caller_id.as_ref()
    }

    fn trace_id(&self) -> &str {
        &self.traceparent
    }

    fn request_id(&self) -> &str {
        &self.request_id
    }

    // ========== RPC 通信 ==========

    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: &[u8],
        timeout_ms: i64,
    ) -> ActorResult<Vec<u8>> {
        let request_id = js_sys::Math::random().to_string();

        // 通过 bridge 注册 pending RPC（使 handle_fast_path 能识别为响应而非入站请求）
        if let Some(bridge) = &self.bridge {
            bridge.register_pending_rpc(request_id.clone());
            // 确保与目标的 WebRTC 连接已就绪
            bridge.ensure_connection(target).await?;
        }

        let envelope = RpcEnvelope {
            route_key: route_key.to_string(),
            payload: Some(Bytes::from(payload.to_vec())),
            error: None,
            traceparent: Some(self.traceparent.clone()),
            tracestate: Some(self.tracestate.clone()),
            request_id,
            metadata: vec![],
            timeout_ms,
        };

        let response_bytes = self.outproc_gate.send_request(target, envelope).await?;
        Ok(response_bytes.to_vec())
    }

    async fn discover(&self, target_type: &ActrType) -> ActorResult<ActrId> {
        match &self.bridge {
            Some(bridge) => bridge.discover_target(target_type).await,
            None => Err(actr_protocol::ProtocolError::TransportError(
                "RuntimeBridge not available for discover".to_string(),
            )),
        }
    }

    // ========== 类型安全通信方法 ==========

    async fn call<R: RpcRequest>(&self, target: &ActrId, request: R) -> ActorResult<R::Response> {
        // 1. 编码请求为 protobuf bytes
        let payload: Bytes = request.encode_to_vec().into();

        // 2. 从 RpcRequest trait 获取 route_key
        let route_key = R::route_key().to_string();

        // 3. 构造 RpcEnvelope（继承当前 Context 的追踪信息）
        let envelope = RpcEnvelope {
            route_key,
            payload: Some(payload),
            error: None,
            traceparent: Some(self.traceparent.clone()),
            tracestate: Some(self.tracestate.clone()),
            request_id: js_sys::Math::random().to_string(), // 简化 ID 生成
            metadata: vec![],
            timeout_ms: 30000,
        };

        // 4. 通过 OutGate 发送
        let response_bytes = self.outproc_gate.send_request(target, envelope).await?;

        // 5. 解码响应
        R::Response::decode(&*response_bytes).map_err(|e| {
            actr_protocol::ProtocolError::Actr(actr_protocol::ActrError::DecodeFailure {
                message: format!(
                    "Failed to decode {}: {}",
                    std::any::type_name::<R::Response>(),
                    e
                ),
            })
        })
    }

    async fn tell<R: RpcRequest>(&self, target: &ActrId, message: R) -> ActorResult<()> {
        // 1. 编码消息
        let payload: Bytes = message.encode_to_vec().into();

        // 2. 获取 route_key
        let route_key = R::route_key().to_string();

        // 3. 构造 RpcEnvelope（fire-and-forget 语义）
        let envelope = RpcEnvelope {
            route_key,
            payload: Some(payload),
            error: None,
            traceparent: Some(self.traceparent.clone()),
            tracestate: Some(self.tracestate.clone()),
            request_id: js_sys::Math::random().to_string(), // 简化 ID 生成
            metadata: vec![],
            timeout_ms: 0, // 0 表示不等待响应
        };

        // 4. 通过 OutGate 发送
        self.outproc_gate.send_message(target, envelope).await
    }

    // ========== Stream 注册方法 ==========
    //
    // 注意：Stream 注册在 SW 端主要用于向 DOM 转发注册请求
    // 实际的回调执行在 DOM 端的 Fast Path

    async fn register_stream(
        &self,
        stream_id: String,
        _callback: Box<dyn FnMut(Bytes) + 'static>,
    ) -> ActorResult<()> {
        // TODO: 通过 PostMessage 将注册请求发送到 DOM 端
        // DOM 端的 StreamHandlerRegistry 会管理实际的回调
        log::info!(
            "[Context] register_stream: {} (forwarding to DOM)",
            stream_id
        );

        // 暂时只记录，完整实现需要 SW ↔ DOM 通信管道
        // 当前阶段返回成功，让调用者知道接口存在
        Ok(())
    }

    async fn unregister_stream(&self, stream_id: &str) -> ActorResult<()> {
        // TODO: 通过 PostMessage 将注销请求发送到 DOM 端
        log::info!(
            "[Context] unregister_stream: {} (forwarding to DOM)",
            stream_id
        );
        Ok(())
    }

    async fn register_media_track(
        &self,
        track_id: String,
        _callback: Box<dyn FnMut(Bytes) + 'static>,
    ) -> ActorResult<()> {
        // TODO: 通过 PostMessage 将注册请求发送到 DOM 端
        // DOM 端的 MediaFrameHandlerRegistry 会管理实际的回调
        log::info!(
            "[Context] register_media_track: {} (forwarding to DOM)",
            track_id
        );
        Ok(())
    }

    async fn unregister_media_track(&self, track_id: &str) -> ActorResult<()> {
        // TODO: 通过 PostMessage 将注销请求发送到 DOM 端
        log::info!(
            "[Context] unregister_media_track: {} (forwarding to DOM)",
            track_id
        );
        Ok(())
    }

    // ========== Stream 发送方法 ==========

    async fn send_media_sample(
        &self,
        target: &ActrId,
        track_id: &str,
        data: Bytes,
    ) -> ActorResult<()> {
        log::debug!(
            "[Context] send_media_sample: target={:?}, track_id={}, size={}",
            target,
            track_id,
            data.len()
        );

        // 构造带 track_id 前缀的数据
        // 格式: [track_id_len(4) | track_id(N) | data(M)]
        let track_id_bytes = track_id.as_bytes();
        let mut payload = Vec::with_capacity(4 + track_id_bytes.len() + data.len());
        payload.extend_from_slice(&(track_id_bytes.len() as u32).to_be_bytes());
        payload.extend_from_slice(track_id_bytes);
        payload.extend_from_slice(&data);

        // 通过 OutGate 发送 Fast Path 数据
        self.outproc_gate
            .send_data_stream(
                target,
                actr_protocol::PayloadType::MediaRtp,
                Bytes::from(payload),
            )
            .await
    }

    async fn send_data_stream(
        &self,
        target: &ActrId,
        stream_id: &str,
        data: Bytes,
    ) -> ActorResult<()> {
        log::debug!(
            "[Context] send_data_stream: target={:?}, stream_id={}, size={}",
            target,
            stream_id,
            data.len()
        );

        // 构造带 stream_id 前缀的数据
        // 格式: [stream_id_len(4) | stream_id(N) | data(M)]
        let stream_id_bytes = stream_id.as_bytes();
        let mut payload = Vec::with_capacity(4 + stream_id_bytes.len() + data.len());
        payload.extend_from_slice(&(stream_id_bytes.len() as u32).to_be_bytes());
        payload.extend_from_slice(stream_id_bytes);
        payload.extend_from_slice(&data);

        // 通过 OutGate 发送 Fast Path 数据（默认使用 STREAM_RELIABLE）
        self.outproc_gate
            .send_data_stream(
                target,
                actr_protocol::PayloadType::StreamReliable,
                Bytes::from(payload),
            )
            .await
    }
}
