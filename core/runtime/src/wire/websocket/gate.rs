//! WebSocketGate - WebSocket 入站连接的消息路由器
//!
//! 从 `WebSocketServer` 通道接收新建连接（含发送方 ActrId bytes 和 AIdCredential），
//! 对每个连接先进行 Ed25519 credential 验签（若已配置 `WsAuthContext`），
//! 验签通过后按 PayloadType 路由消息到 Mailbox 或 DataStreamRegistry。
//!
//! # 设计对比 WebRtcGate
//!
//! | 关注点 | WebRtcGate | WebSocketGate |
//! |--------|-----------|---------------|
//! | 传输层 | WebRTC DataChannel | WebSocket (TCP) |
//! | 发送方认证 | actrix 信令验证 credential | 本地 Ed25519 验签（AisKeyCache） |
//! | 消息聚合 | `WebRtcCoordinator.receive_message()` | 逐连接读取 `DataLane` |

use super::connection::WebSocketConnection;
use super::server::InboundWsConn;
use crate::ais_key_cache::AisKeyCache;
use crate::error::{ActorResult, ActrError};
use crate::inbound::DataStreamRegistry;
use crate::lifecycle::CredentialState;
use crate::wire::webrtc::SignalingClient;
use actr_framework::Bytes;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{AIdCredential, ActrId, ActrIdExt, DataStream, IdentityClaims, PayloadType, RpcEnvelope};
use actr_runtime_mailbox::{Mailbox, MessagePriority};
use ed25519_dalek::{Signature, Verifier as Ed25519Verifier};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};

/// WebSocket 身份验证上下文（可选）
///
/// 配置后，gate 将对每个入站连接执行 Ed25519 credential 验签：
/// - 验签失败的连接直接丢弃，不启动 lane reader
/// - 连接方未携带 credential 时，视同验签失败
pub struct WsAuthContext {
    /// AIS signing 公钥缓存（本地命中直接验签，miss 时通过 signaling 拉取）
    pub ais_key_cache: Arc<AisKeyCache>,
    /// 本机 ActrId（cache miss 时向 signaling 请求公钥所需）
    pub actor_id: ActrId,
    /// 本机凭证状态（cache miss 时向 signaling 认证所需）
    pub credential_state: CredentialState,
    /// Signaling 客户端（cache miss 时拉取公钥）
    pub signaling_client: Arc<dyn SignalingClient>,
}

/// WebSocketGate - 接收并路由入站 WebSocket 消息
pub struct WebSocketGate {
    /// 入站连接通道（take 一次后 move 进 background task）
    conn_rx: tokio::sync::Mutex<Option<mpsc::Receiver<InboundWsConn>>>,

    /// 待响应请求表（request_id → (caller_id, oneshot::Sender)）
    /// **与 OutprocOutGate 共享**，以便正确路由 Response
    pending_requests:
        Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>,

    /// DataStream 注册表（fast-path 流消息路由）
    data_stream_registry: Arc<DataStreamRegistry>,

    /// 入站连接身份验证上下文
    auth_ctx: Option<Arc<WsAuthContext>>,
}

impl WebSocketGate {
    /// 创建 WebSocketGate
    ///
    /// # Arguments
    /// - `conn_rx`: 来自 `WebSocketServer::bind()` 的接收端
    /// - `pending_requests`: 与 OutprocOutGate 共享的待响应表
    /// - `data_stream_registry`: DataStream 注册表
    /// - `auth_ctx`: 身份验证上下文（配置后对所有入站连接强制验签）
    pub fn new(
        conn_rx: mpsc::Receiver<InboundWsConn>,
        pending_requests: Arc<
            RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>,
        >,
        data_stream_registry: Arc<DataStreamRegistry>,
        auth_ctx: Option<WsAuthContext>,
    ) -> Self {
        Self {
            conn_rx: tokio::sync::Mutex::new(Some(conn_rx)),
            pending_requests,
            data_stream_registry,
            auth_ctx: auth_ctx.map(Arc::new),
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

    /// 验证入站连接的 AIdCredential Ed25519 签名
    ///
    /// 返回 `Some(verified_actor_id_str)` 表示验签通过，`None` 表示失败（已记录日志）。
    /// `source_id_bytes` 为 `X-Actr-Source-ID` 提供的 ActrId protobuf bytes。
    async fn verify_credential(
        credential: &AIdCredential,
        source_id_bytes: &[u8],
        auth_ctx: &WsAuthContext,
    ) -> Option<()> {
        // 取得 actor B 自身的 ActrId 和 credential（用于 cache miss 拉取公钥时向 signaling 认证）
        let local_credential = auth_ctx.credential_state.credential().await;

        // 从 AisKeyCache 获取 key_id 对应的 verifying key（本地命中或从 signaling 拉取）
        let verifying_key = match auth_ctx.ais_key_cache
            .get_or_fetch(
                credential.key_id,
                &auth_ctx.actor_id,
                &local_credential,
                auth_ctx.signaling_client.as_ref(),
            )
            .await
        {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    key_id = credential.key_id,
                    error = ?e,
                    "⚠️ WS credential 验签失败：无法获取 signing key"
                );
                return None;
            }
        };

        // Ed25519 验签
        let sig_result = credential.signature[..]
            .try_into()
            .ok()
            .and_then(|sig_bytes: [u8; 64]| {
                let signature = Signature::from_bytes(&sig_bytes);
                verifying_key.verify(&credential.claims[..], &signature).ok()
            });
        if sig_result.is_none() {
            tracing::warn!(
                key_id = credential.key_id,
                "⚠️ WS AIdCredential Ed25519 验签失败"
            );
            return None;
        }

        // 解码 IdentityClaims
        let claims = match IdentityClaims::decode(&credential.claims[..]) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(key_id = credential.key_id, error = ?e, "⚠️ WS IdentityClaims proto 解码失败");
                return None;
            }
        };

        // 检查 expires_at
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if claims.expires_at <= now {
            tracing::warn!(
                key_id = credential.key_id,
                expires_at = claims.expires_at,
                "⚠️ WS AIdCredential 已过期"
            );
            return None;
        }

        // 校验 claims.actor_id 与 X-Actr-Source-ID 一致（防止身份声称不一致）
        match ActrId::decode(source_id_bytes) {
            Ok(source_actor_id) => {
                let source_repr = source_actor_id.to_string_repr();
                if claims.actor_id != source_repr {
                    tracing::warn!(
                        claimed = %claims.actor_id,
                        source_id = %source_repr,
                        "⚠️ WS credential actor_id 与 X-Actr-Source-ID 不一致，拒绝连接"
                    );
                    return None;
                }
                tracing::info!(
                    actor_id = %claims.actor_id,
                    "✅ WS 入站连接身份验证通过"
                );
            }
            Err(e) => {
                tracing::warn!(error = ?e, "⚠️ WS X-Actr-Source-ID 解码失败，拒绝连接");
                return None;
            }
        }

        Some(())
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
    /// 若配置了 `auth_ctx`，则对每个入站连接先进行 credential 验签，
    /// 验签失败则丢弃连接；验签通过后调用 `spawn_connection_tasks`。
    pub async fn start_receive_loop(&self, mailbox: Arc<dyn Mailbox>) -> ActorResult<()> {
        let rx = self.conn_rx.lock().await.take().ok_or_else(|| {
            ActrError::Internal("WebSocketGate: start_receive_loop already called".to_string())
        })?;

        let pending_requests = self.pending_requests.clone();
        let data_stream_registry = self.data_stream_registry.clone();
        let auth_ctx = self.auth_ctx.clone();

        tokio::spawn(async move {
            tracing::info!("🚀 WebSocketGate receive loop started");

            let mut rx = rx;
            while let Some((conn, source_id, credential_opt)) = rx.recv().await {
                tracing::info!(
                    "🔗 WS new inbound connection (source_id len={}, has_credential={})",
                    source_id.len(),
                    credential_opt.is_some()
                );

                // Credential 验签（若已配置 auth_ctx）
                if let Some(ref ctx) = auth_ctx {
                    match credential_opt {
                        Some(ref credential) => {
                            if Self::verify_credential(credential, &source_id, ctx).await.is_none() {
                                tracing::warn!(
                                    "⚠️ WS 入站连接 credential 验签失败，丢弃连接"
                                );
                                continue; // 丢弃连接，继续等待下一个
                            }
                        }
                        None => {
                            tracing::warn!(
                                "⚠️ WS 入站连接未携带 X-Actr-Credential，拒绝连接（auth_ctx 已配置）"
                            );
                            continue;
                        }
                    }

                    Self::spawn_connection_tasks(
                        conn,
                        source_id,
                        pending_requests.clone(),
                        data_stream_registry.clone(),
                        mailbox.clone(),
                    );
                } else {
                    tracing::error!("❌ WS auth_ctx 未配置，拒绝连接（配置错误）");
                }
            }

            tracing::info!("🔌 WebSocketGate receive loop exited");
        });

        Ok(())
    }
}
