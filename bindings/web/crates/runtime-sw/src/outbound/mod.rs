//! Outbound Layer - 出站消息发送
//!
//! 对标 actr 的 outbound 层，提供统一的发送接口
//!
//! # 出站路径
//!
//! ```text
//! Actor ctx.call()/tell()
//!   → OutGate::OutprocOut
//!     → OutprocOutGate (ActrId→Dest 映射 + pending_requests)
//!       → OutprocTransportManager (Dest→DestTransport 映射)
//!         → DestTransport (事件驱动发送循环)
//!           → WirePool (优先级选择: WebRTC > WebSocket)
//!             → WireHandle::WebRTC.get_lane()
//!               → DataLane::PostMessage { port: 专用 MessagePort }
//!                 → port.postMessage(data)  [零拷贝, 无命令协议]
//!                   → DOM bridge → RtcDataChannel.send() → Remote
//! ```

mod inproc_out_gate;
mod outproc_out_gate;

pub use inproc_out_gate::InprocOutGate;
pub use outproc_out_gate::OutprocOutGate;

use actr_protocol::{ActorResult, ActrId, PayloadType, RpcEnvelope};
use bytes::Bytes;
use std::sync::Arc;

/// OutGate - 出站消息门枚举
///
/// # 变体
///
/// - **InprocOut**: SW 内部 Actor 之间的通信（零序列化）
/// - **OutprocOut**: 跨节点传输（通过专用 MessagePort + 完整传输栈）
#[derive(Clone)]
pub enum OutGate {
    /// InprocOut - SW 内部通信（零序列化）
    InprocOut(Arc<InprocOutGate>),

    /// OutprocOut - 跨节点传输
    ///
    /// OutprocOutGate → OutprocTransportManager → DestTransport
    ///   → WirePool → WireHandle → DataLane::PostMessage（专用 MessagePort 直接发送）
    OutprocOut(Arc<OutprocOutGate>),
}

impl OutGate {
    /// 创建 InprocOut gate
    pub fn inproc(gate: Arc<InprocOutGate>) -> Self {
        Self::InprocOut(gate)
    }

    /// 创建 OutprocOut gate
    pub fn outproc(gate: Arc<OutprocOutGate>) -> Self {
        Self::OutprocOut(gate)
    }

    /// 发送请求并等待响应
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        match self {
            OutGate::InprocOut(gate) => gate.send_request(target, envelope).await,
            OutGate::OutprocOut(gate) => gate.send_request(target, envelope).await,
        }
    }

    /// 发送单向消息（不等待响应）
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(gate) => gate.send_message(target, envelope).await,
            OutGate::OutprocOut(gate) => gate.send_message(target, envelope).await,
        }
    }

    /// 发送 DataStream（Fast Path）
    pub async fn send_data_stream(
        &self,
        target: &ActrId,
        payload_type: PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(gate) => gate.send_data_stream(target, payload_type, data).await,
            OutGate::OutprocOut(gate) => gate.send_data_stream(target, payload_type, data).await,
        }
    }

    /// 尝试处理远程响应
    ///
    /// 检查此 OutGate 是否有对应 request_id 的 pending request。
    /// 如果有，resolve 并返回 true；否则返回 false。
    pub fn try_handle_response(&self, request_id: &str, response: Bytes) -> bool {
        match self {
            OutGate::InprocOut(_) => false,
            OutGate::OutprocOut(gate) => gate.handle_response(request_id.to_string(), response),
        }
    }
}
