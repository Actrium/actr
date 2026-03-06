//! InprocOutGate - 进程内传输适配器（出站）
//!
//! Web 版本的 InprocOutGate，用于 SW 内部 Actor 之间的通信

use actr_protocol::{ActorResult, ActrError, ActrId, PayloadType, RpcEnvelope};
use bytes::Bytes;
use futures::channel::oneshot;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use actr_framework::MediaSample;
/// InprocOutGate - 进程内传输适配器
///
/// # 设计说明
///
/// Web 版本的 InprocOutGate 与 actr 版本类似，但有以下区别：
/// - 使用 JS 环境的异步原语（futures::channel）
/// - 不需要 mpsc channel（因为 SW 是单线程的）
/// - 通过 request_id 映射实现请求-响应模式
///
/// # 通信模式
///
/// 1. **请求-响应（send_request）**：
///    - 创建 oneshot channel
///    - 将 request_id → sender 注册到 pending_requests
///    - 发送请求到目标 Actor
///    - 等待响应
///
/// 2. **单向消息（send_message）**：
///    - 直接发送，不等待响应
///
/// 3. **DataStream（Fast Path）**：
///    - 绕过序列化，直接传递 bytes
pub struct InprocOutGate {
    /// Pending requests: request_id → oneshot sender
    pending_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Bytes>>>>,

    /// 消息处理回调（由 System 设置）
    /// 接收 (target_id, envelope) 并将其路由到目标 Actor
    message_handler: Arc<Mutex<Option<MessageHandler>>>,
}

/// 消息处理回调类型
/// 注意：WASM/Service Worker 是单线程环境，不需要 Send + Sync
pub type MessageHandler = Box<dyn Fn(ActrId, RpcEnvelope)>;

impl InprocOutGate {
    /// 创建新的 InprocOutGate
    pub fn new() -> Self {
        Self {
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            message_handler: Arc::new(Mutex::new(None)),
        }
    }

    /// 设置消息处理回调
    ///
    /// # 参数
    /// - `handler`: 接收 (target_id, envelope) 的回调函数
    ///
    /// # 用途
    /// System 初始化时调用，将消息路由到对应的 Actor
    pub fn set_message_handler<F>(&self, handler: F)
    where
        F: Fn(ActrId, RpcEnvelope) + 'static,
    {
        let mut guard = self.message_handler.lock();
        *guard = Some(Box::new(handler));
    }

    /// 发送请求并等待响应
    ///
    /// # 实现说明
    /// 1. 创建 oneshot channel
    /// 2. 注册 pending request
    /// 3. 调用 message_handler 发送请求
    /// 4. 等待响应
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        log::debug!(
            "📤 InprocOutGate::send_request to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // 1. 创建 oneshot channel
        let (tx, rx) = oneshot::channel();

        // 2. 注册 pending request
        {
            let mut pending = self.pending_requests.lock();
            pending.insert(envelope.request_id.clone(), tx);
        }

        // 3. 发送请求到目标 Actor
        {
            let guard = self.message_handler.lock();
            match guard.as_ref() {
                Some(handler) => {
                    handler(target.clone(), envelope);
                }
                None => {
                    // 清理 pending request
                    drop(guard); // 释放锁
                    self.pending_requests.lock().remove(&envelope.request_id);

                    return Err(ActrError::Unavailable(
                        "InprocOutGate message_handler not set".to_string(),
                    ));
                }
            }
        }

        // 4. 等待响应
        let response = rx
            .await
            .map_err(|_| ActrError::Unavailable("Response channel closed".to_string()))?;

        Ok(response)
    }

    /// 发送单向消息（不等待响应）
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        log::debug!(
            "📤 InprocOutGate::send_message to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // 获取 message_handler 并调用
        let guard = self.message_handler.lock();
        match guard.as_ref() {
            Some(handler) => {
                handler(target.clone(), envelope);
                Ok(())
            }
            None => Err(ActrError::Unavailable(
                "InprocOutGate message_handler not set".to_string(),
            )),
        }
    }

    /// 发送 DataStream（Fast Path）
    ///
    /// # 参数
    /// - `target`: 目标 Actor ID
    /// - `payload_type`: PayloadType (StreamReliable 或 StreamLatencyFirst)
    /// - `data`: 序列化后的 DataStream bytes
    pub async fn send_data_stream(
        &self,
        target: &ActrId,
        _payload_type: PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        log::debug!(
            "📤 InprocOutGate::send_data_stream to {:?}, size={} bytes",
            target,
            data.len()
        );

        // 暂时通过 RpcEnvelope 发送（未来可优化为 Fast Path）
        let envelope = RpcEnvelope {
            route_key: "__fast_path_data_stream__".to_string(),
            payload: Some(data),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: format!("ds-{}", js_sys::Math::random()),
            metadata: vec![],
            timeout_ms: 0,
        };

        self.send_message(target, envelope).await
    }

    /// 发送 MediaSample（Fast Path）
    ///
    /// # 参数
    /// - `target`: 目标 Actor ID
    /// - `track_id`: Track ID
    /// - `sample`: Media sample
    pub async fn send_media_sample(
        &self,
        target: &ActrId,
        track_id: &str,
        _sample: MediaSample,
    ) -> ActorResult<()> {
        log::warn!(
            "⚠️  InprocOutGate::send_media_sample to {:?}, track={} - not implemented",
            target,
            track_id
        );

        Err(ActrError::NotImplemented(
            "send_media_sample not yet implemented for Web InprocOutGate".to_string(),
        ))
    }

    /// 处理接收到的响应
    ///
    /// # 用途
    /// System 收到响应时调用，匹配 pending request 并发送响应
    pub fn handle_response(&self, request_id: &str, response: Bytes) {
        let mut pending = self.pending_requests.lock();
        if let Some(tx) = pending.remove(request_id) {
            let _ = tx.send(response); // 忽略错误（接收方可能已取消）
        } else {
            log::warn!("Received response for unknown request_id: {}", request_id);
        }
    }
}

impl Default for InprocOutGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inproc_out_gate_creation() {
        let _gate = InprocOutGate::new();
    }

    #[test]
    fn test_handle_response_unknown_request() {
        let gate = InprocOutGate::new();
        gate.handle_response("unknown-id", Bytes::from("test"));
        // 应该只输出 warning，不应该 panic
    }
}
