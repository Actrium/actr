//! Outbound Layer - 出站消息发送
//!
//! 对标 actr 的 outbound 层，提供统一的发送接口
//!
//! # 出站路径
//!
//! ```text
//! Actor ctx.call()/tell()
//!   → Gate::Peer
//!     → PeerGate (ActrId→Dest 映射 + pending_requests)
//!       → PeerTransport (Dest→DestTransport 映射)
//!         → DestTransport (事件驱动发送循环)
//!           → WirePool (优先级选择: WebRTC > WebSocket)
//!             → WireHandle::WebRTC.get_lane()
//!               → DataLane::PostMessage { port: 专用 MessagePort }
//!                 → port.postMessage(data)  [零拷贝, 无命令协议]
//!                   → DOM bridge → RtcDataChannel.send() → Remote
//! ```

mod host_gate;
mod peer_gate;

pub use host_gate::HostGate;
pub use peer_gate::PeerGate;

use actr_protocol::{ActorResult, ActrId, PayloadType, RpcEnvelope};
use bytes::Bytes;
use std::sync::Arc;

/// Gate - 出站消息门枚举
///
/// # 变体
///
/// - **Host**: SW 内部 Actor 之间的通信（零序列化）
/// - **Peer**: 跨节点传输（通过专用 MessagePort + 完整传输栈）
#[derive(Clone)]
pub enum Gate {
    /// Host - SW 内部通信（零序列化）
    Host(Arc<HostGate>),

    /// Peer - 跨节点传输
    ///
    /// PeerGate → PeerTransport → DestTransport
    ///   → WirePool → WireHandle → DataLane::PostMessage（专用 MessagePort 直接发送）
    Peer(Arc<PeerGate>),
}

impl Gate {
    /// 创建 Host gate
    pub fn host(gate: Arc<HostGate>) -> Self {
        Self::Host(gate)
    }

    /// 创建 Peer gate
    pub fn peer(gate: Arc<PeerGate>) -> Self {
        Self::Peer(gate)
    }

    /// 发送请求并等待响应
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        match self {
            Gate::Host(gate) => gate.send_request(target, envelope).await,
            Gate::Peer(gate) => gate.send_request(target, envelope).await,
        }
    }

    /// 发送单向消息（不等待响应）
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        match self {
            Gate::Host(gate) => gate.send_message(target, envelope).await,
            Gate::Peer(gate) => gate.send_message(target, envelope).await,
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
            Gate::Host(gate) => gate.send_data_stream(target, payload_type, data).await,
            Gate::Peer(gate) => gate.send_data_stream(target, payload_type, data).await,
        }
    }

    /// 尝试处理远程响应
    ///
    /// 检查此 Gate 是否有对应 request_id 的 pending request。
    /// 如果有，resolve 并返回 true；否则返回 false。
    pub fn try_handle_response(&self, request_id: &str, response: Bytes) -> bool {
        match self {
            Gate::Host(_) => false,
            Gate::Peer(gate) => gate.handle_response(request_id.to_string(), response),
        }
    }
}
