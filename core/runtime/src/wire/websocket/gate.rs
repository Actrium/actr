//! WebSocketGate - WebSocket 入站连接的消息路由器
//!
//! 从 `WebSocketServer` 通道接收新建连接（含发送方 ActrId bytes），
//! 对每个连接按 PayloadType 读取到来的帧，并路由到 Mailbox 或 DataStreamRegistry。
//!
//! # 设计对比 WebRtcGate
//!
//! | 关注点 | WebRtcGate | WebSocketGate |
//! |--------|-----------|---------------|
//! | 传输层 | WebRTC DataChannel | WebSocket (TCP) |
//! | 消息聚合 | `WebRtcCoordinator.receive_message()` | 逐连接读取 `DataLane` |
//! | 发送方识别 | WebRTC 对端 ID（连接级别） | HTTP 头 `X-Actr-Source-ID` |
//! | 多路复用 | 单 coordinator，多 peer | 每连接独立 `WebSocketConnection` |

use super::connection::WebSocketConnection;
use crate::error::{ActorResult, ActrError};
use crate::inbound::DataStreamRegistry;
use actr_framework::Bytes;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActrId, ActrIdExt, DataStream, PayloadType, RpcEnvelope};
use actr_runtime_mailbox::{Mailbox, MessagePriority};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};

/// WebSocketGate - 接收并路由入站 WebSocket 消息
pub struct WebSocketGate {
    /// 入站连接通道（take 一次后 move 进 background task）
    conn_rx: tokio::sync::Mutex<Option<mpsc::Receiver<(WebSocketConnection, Vec<u8>)>>>,

    /// 待响应请求表（request_id → (caller_id, oneshot::Sender)）
    /// **与 OutprocOutGate 共享**，以便正确路由 Response
    pending_requests:
        Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>,

    /// DataStream 注册表（fast-path 流消息路由）
    data_stream_registry: Arc<DataStreamRegistry>,
}

impl WebSocketGate {
    /// 创建 WebSocketGate
    ///
    /// # Arguments
    /// - `conn_rx`: 来自 `WebSocketServer::bind()` 的接收端
    /// - `pending_requests`: 与 OutprocOutGate 共享的待响应表
    /// - `data_stream_registry`: DataStream 注册表
    pub fn new(
        conn_rx: mpsc::Receiver<(WebSocketConnection, Vec<u8>)>,
        pending_requests: Arc<
            RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>,
        >,
        data_stream_registry: Arc<DataStreamRegistry>,
    ) -> Self {
        Self {
            conn_rx: tokio::sync::Mutex::new(Some(conn_rx)),
            pending_requests,
            data_stream_registry,
        }
    }

    /// 处理 RpcEnvelope：Response 唤醒等待方，Request 入队 Mailbox
    async fn handle_envelope(
        envelope: RpcEnvelope,
        from_bytes: Vec<u8>,
        data: Bytes,
        payload_type: PayloadType,
        pending_requests: Arc<
            RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>,
        >,
        mailbox: Arc<dyn Mailbox>,
    ) {
        let request_id = envelope.request_id.clone();

        let mut pending = pending_requests.write().await;
        if let Some((target, response_tx)) = pending.remove(&request_id) {
            drop(pending);
            tracing::debug!(
                "📬 WS Received RPC Response: request_id={}, target={}",
                request_id,
                target.to_string_repr()
            );

            let result = match (envelope.payload, envelope.error) {
                (Some(payload), None) => Ok(payload),
                (None, Some(error)) => Err(ActrError::Unavailable(format!(
                    "RPC error {}: {}",
                    error.code, error.message
                ))),
                _ => Err(ActrError::DecodeFailure(
                    "Invalid RpcEnvelope: payload and error fields inconsistent".to_string(),
                )),
            };
            let _ = response_tx.send(result);
        } else {
            drop(pending);
            tracing::debug!("📥 WS Received RPC Request: request_id={}", request_id);

            let priority = match payload_type {
                PayloadType::RpcSignal => MessagePriority::High,
                _ => MessagePriority::Normal,
            };

            match mailbox.enqueue(from_bytes, data.to_vec(), priority).await {
                Ok(msg_id) => {
                    tracing::debug!(
                        "✅ WS RPC message enqueued: msg_id={}, priority={:?}",
                        msg_id,
                        priority
                    );
                }
                Err(e) => {
                    tracing::error!("❌ WS Mailbox enqueue failed: {:?}", e);
                }
            }
        }
    }

    /// 为单个 WebSocket 连接启动接收任务
    ///
    /// 逐一读取 `PayloadType::RpcReliable`、`RpcSignal`、`StreamReliable`、
    /// `StreamLatencyFirst` 四条 lane，对每条 lane 起一个独立 task。
    fn spawn_connection_tasks(
        conn: WebSocketConnection,
        source_id: Vec<u8>,
        pending_requests: Arc<
            RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>,
        >,
        data_stream_registry: Arc<DataStreamRegistry>,
        mailbox: Arc<dyn Mailbox>,
    ) {
        // Spawn per-PayloadType receive tasks
        for pt in [
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
        ] {
            let conn_clone = conn.clone();
            let src = source_id.clone();
            let pending = pending_requests.clone();
            let registry = data_stream_registry.clone();
            let mb = mailbox.clone();

            tokio::spawn(async move {
                // get_lane lazily creates the mpsc channel and registers in router
                let lane = match conn_clone.get_lane(pt).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("❌ WS get_lane({:?}) failed: {:?}", pt, e);
                        return;
                    }
                };

                tracing::debug!("📡 WS lane reader started for {:?}", pt);

                loop {
                    match lane.recv().await {
                        Ok(data) => {
                            let data_bytes = Bytes::copy_from_slice(&data);

                            match pt {
                                PayloadType::RpcReliable | PayloadType::RpcSignal => {
                                    match RpcEnvelope::decode(&data[..]) {
                                        Ok(envelope) => {
                                            Self::handle_envelope(
                                                envelope,
                                                src.clone(),
                                                data_bytes,
                                                pt,
                                                pending.clone(),
                                                mb.clone(),
                                            )
                                            .await;
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "❌ WS Failed to decode RpcEnvelope: {:?}",
                                                e
                                            );
                                        }
                                    }
                                }
                                PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                                    match DataStream::decode(&data[..]) {
                                        Ok(chunk) => {
                                            tracing::debug!(
                                                "📦 WS Received DataStream: stream_id={}, seq={}",
                                                chunk.stream_id,
                                                chunk.sequence,
                                            );
                                            match ActrId::decode(&src[..]) {
                                                Ok(sender_id) => {
                                                    registry.dispatch(chunk, sender_id).await;
                                                }
                                                Err(e) => {
                                                    tracing::error!(
                                                        "❌ WS Failed to decode sender ActrId: {:?}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "❌ WS Failed to decode DataStream: {:?}",
                                                e
                                            );
                                        }
                                    }
                                }
                                PayloadType::MediaRtp => {
                                    tracing::warn!(
                                        "⚠️ MediaRtp received in WebSocketGate (unexpected)"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::info!("🔌 WS lane {:?} closed: {:?}", pt, e);
                            break;
                        }
                    }
                }

                tracing::debug!("📡 WS lane reader exited for {:?}", pt);
            });
        }
    }

    /// 启动连接接受循环（由 ActrNode 调用，只能调用一次）
    ///
    /// 内部 take 出 `conn_rx`，move 进 background task 以无锁接收新连接。
    /// 对每个入站连接调用 `spawn_connection_tasks` 起 per-lane 接收 task。
    pub async fn start_receive_loop(&self, mailbox: Arc<dyn Mailbox>) -> ActorResult<()> {
        let rx = self.conn_rx.lock().await.take().ok_or_else(|| {
            ActrError::Internal("WebSocketGate: start_receive_loop already called".to_string())
        })?;

        let pending_requests = self.pending_requests.clone();
        let data_stream_registry = self.data_stream_registry.clone();

        tokio::spawn(async move {
            tracing::info!("🚀 WebSocketGate receive loop started");

            let mut rx = rx;
            while let Some((conn, source_id)) = rx.recv().await {
                tracing::info!(
                    "🔗 WS new inbound connection (source_id len={})",
                    source_id.len()
                );
                Self::spawn_connection_tasks(
                    conn,
                    source_id,
                    pending_requests.clone(),
                    data_stream_registry.clone(),
                    mailbox.clone(),
                );
            }

            tracing::info!("🔌 WebSocketGate receive loop exited");
        });

        Ok(())
    }
}
