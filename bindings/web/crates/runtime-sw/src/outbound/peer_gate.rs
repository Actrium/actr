//! PeerGate - 跨节点传输适配器（出站）
//!
//! 封装 PeerTransport，提供标准的 Actor 发送接口

use crate::transport::PeerTransport;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrError, ActrId, PayloadType, RpcEnvelope};
use actr_web_common::Dest;
use bytes::Bytes;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

/// PeerGate - 跨节点传输适配器
///
/// # 职责
/// - 封装 PeerTransport
/// - 提供 ActrId → Dest 映射
/// - 实现请求-响应模式（oneshot channel）
pub struct PeerGate {
    /// Transport manager
    transport: Arc<PeerTransport>,

    /// ActrId → Dest 映射
    /// 用于将 ActrId 转换为网络目标
    actor_dest_map: Arc<Mutex<HashMap<ActrId, Dest>>>,

    /// Pending requests: request_id → oneshot sender
    pending_requests: Arc<Mutex<HashMap<String, futures::channel::oneshot::Sender<Bytes>>>>,
}

impl PeerGate {
    /// 创建新的 PeerGate
    pub fn new(transport: Arc<PeerTransport>) -> Self {
        Self {
            transport,
            actor_dest_map: Arc::new(Mutex::new(HashMap::new())),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 注册 ActrId → Dest 映射
    ///
    /// # 用途
    /// 在知道 Actor 的网络位置时调用
    pub fn register_actor(&self, actor_id: ActrId, dest: Dest) {
        let mut map = self.actor_dest_map.lock();
        log::debug!("Registering actor mapping: {:?} → {:?}", &actor_id, &dest);
        map.insert(actor_id, dest);
    }

    /// 获取 ActrId 对应的 Dest
    fn get_dest(&self, actor_id: &ActrId) -> ActorResult<Dest> {
        let map = self.actor_dest_map.lock();
        map.get(actor_id)
            .cloned()
            .ok_or_else(|| ActrError::NotFound(format!("Actor not found: {:?}", actor_id)))
    }

    /// 发送请求并等待响应
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        log::debug!(
            "PeerGate::send_request to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // 1. 获取目标 Dest
        let dest = self.get_dest(target)?;

        // 2. 创建 oneshot channel
        let (tx, rx) = futures::channel::oneshot::channel();

        // 3. 注册 pending request
        {
            let mut pending = self.pending_requests.lock();
            pending.insert(envelope.request_id.clone(), tx);
        }

        // 4. 序列化 envelope 并发送
        let payload = envelope.encode_to_vec();
        self.transport
            .send(&dest, PayloadType::RpcReliable, &payload)
            .await
            .map_err(|e| ActrError::Unavailable(format!("Send failed: {}", e)))?;

        // 5. 等待响应
        let response = rx
            .await
            .map_err(|_| ActrError::Unavailable("Response channel closed".to_string()))?;

        Ok(response)
    }

    /// 发送单向消息（不等待响应）
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        log::debug!(
            "PeerGate::send_message to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // 1. 获取目标 Dest
        let dest = self.get_dest(target)?;

        // 2. 序列化 envelope 并发送（使用 RpcSignal 表示单向）
        let payload = envelope.encode_to_vec();
        self.transport
            .send(&dest, PayloadType::RpcSignal, &payload)
            .await
            .map_err(|e| ActrError::Unavailable(format!("Send failed: {}", e)))?;

        Ok(())
    }

    /// 发送 DataStream（Fast Path）
    pub async fn send_data_stream(
        &self,
        target: &ActrId,
        payload_type: PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        log::debug!(
            "PeerGate::send_data_stream to {:?}, type={:?}",
            target,
            payload_type
        );

        // 1. 获取目标 Dest
        let dest = self.get_dest(target)?;

        // 2. 直接发送 DataStream
        self.transport
            .send(&dest, payload_type, &data)
            .await
            .map_err(|e| ActrError::Unavailable(format!("Send failed: {}", e)))?;

        Ok(())
    }

    /// 处理接收到的响应
    ///
    /// # 用途
    /// InboundPacketDispatcher 收到响应时调用
    ///
    /// 返回 `true` 表示该 request_id 已被成功匹配并处理
    pub fn handle_response(&self, request_id: String, response: Bytes) -> bool {
        let mut pending = self.pending_requests.lock();
        if let Some(tx) = pending.remove(&request_id) {
            let _ = tx.send(response); // 忽略错误（接收方可能已取消）
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::WebWireBuilder;

    #[test]
    fn test_peer_gate_creation() {
        let wire_builder = Arc::new(WebWireBuilder::new());
        let manager = Arc::new(PeerTransport::new(
            "test-sw".to_string(),
            wire_builder,
        ));
        let _gate = PeerGate::new(manager);
    }
}
