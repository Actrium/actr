//! WebSocketGate - message router for inbound WebSocket connections
//!
//! Receives new connections from the `WebSocketServer` channel (including sender ActrId bytes and AIdCredential),
//! performs Ed25519 credential verification for each connection (if `WsAuthContext` is configured),
//! then routes messages by PayloadType to Mailbox or DataStreamRegistry upon successful verification.
//!
//! # Design comparison with WebRtcGate
//!
//! | Concern | WebRtcGate | WebSocketGate |
//! |---------|-----------|---------------|
//! | Transport | WebRTC DataChannel | WebSocket (TCP) |
//! | Sender auth | actrix signaling verifies credential | Local Ed25519 verification (AisKeyCache) |
//! | Message aggregation | `WebRtcCoordinator.receive_message()` | Per-connection `DataLane` reading |

use super::connection::WebSocketConnection;
use super::server::InboundWsConn;
use crate::inbound::DataStreamRegistry;
use crate::key_cache::AisKeyCache;
use crate::lifecycle::CredentialState;
use crate::wire::SignalingKeyFetcher;
use crate::wire::webrtc::SignalingClient;
use actr_framework::Bytes;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrIdExt, DataStream, IdentityClaims, PayloadType, RpcEnvelope,
};
use actr_protocol::{ActorResult, ActrError};
use actr_runtime_mailbox::{Mailbox, MessagePriority};
use ed25519_dalek::{Signature, Verifier as Ed25519Verifier};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};

/// Pending requests map type: request_id → (target_actor_id, oneshot response sender)
type PendingRequestsMap =
    Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>;

/// WebSocket authentication context (optional)
///
/// When configured, gate will perform Ed25519 credential verification for each inbound connection:
/// - Connections that fail verification are dropped without starting lane readers
/// - Connections without credentials are treated as verification failures
pub struct WsAuthContext {
    /// AIS signing public key cache (local hit verifies directly, miss fetches via signaling)
    pub ais_key_cache: Arc<AisKeyCache>,
    /// Local ActrId (needed when requesting public key from signaling on cache miss)
    pub actor_id: ActrId,
    /// Local credential state (needed for signaling authentication on cache miss)
    pub credential_state: CredentialState,
    /// Signaling client (used to fetch public key on cache miss)
    pub signaling_client: Arc<dyn SignalingClient>,
}

/// WebSocketGate - receives and routes inbound WebSocket messages
pub struct WebSocketGate {
    /// Inbound connection channel (taken once and moved into background task)
    conn_rx: tokio::sync::Mutex<Option<mpsc::Receiver<InboundWsConn>>>,

    /// Pending requests map (request_id -> (caller_id, oneshot::Sender))
    /// **Shared with PeerGate** for correct Response routing
    pending_requests: PendingRequestsMap,

    /// DataStream registry (fast-path stream message routing)
    data_stream_registry: Arc<DataStreamRegistry>,

    /// Inbound connection authentication context
    auth_ctx: Option<Arc<WsAuthContext>>,
}

impl WebSocketGate {
    /// Create WebSocketGate
    ///
    /// # Arguments
    /// - `conn_rx`: receiver end from `WebSocketServer::bind()`
    /// - `pending_requests`: pending requests map shared with PeerGate
    /// - `data_stream_registry`: DataStream registry
    /// - `auth_ctx`: authentication context (when configured, enforces credential verification on all inbound connections)
    pub fn new(
        conn_rx: mpsc::Receiver<InboundWsConn>,
        pending_requests: PendingRequestsMap,
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

    /// Handle RpcEnvelope: Response wakes the waiting party, Request enqueues into Mailbox
    async fn handle_envelope(
        envelope: RpcEnvelope,
        from_bytes: Vec<u8>,
        data: Bytes,
        payload_type: PayloadType,
        pending_requests: PendingRequestsMap,
        mailbox: Arc<dyn Mailbox>,
    ) {
        let request_id = envelope.request_id.clone();

        let mut pending = pending_requests.write().await;
        if let Some((target, response_tx)) = pending.remove(&request_id) {
            drop(pending);
            tracing::debug!(
                "📬 WS Received RPC Response: request_id={}, target={}",
                request_id,
                target
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

    /// Verify AIdCredential Ed25519 signature of an inbound connection
    ///
    /// Returns `Some(verified_actor_id_str)` on successful verification, `None` on failure (already logged).
    /// `source_id_bytes` is the ActrId protobuf bytes from `X-Actr-Source-ID`.
    async fn verify_credential(
        credential: &AIdCredential,
        source_id_bytes: &[u8],
        auth_ctx: &WsAuthContext,
    ) -> Option<()> {
        // Get local credential (for signaling authentication on cache miss)
        let local_credential = auth_ctx.credential_state.credential().await;

        // Construct SignalingKeyFetcher adapter, wrapping signaling client as KeyFetcher
        let fetcher = SignalingKeyFetcher {
            client: auth_ctx.signaling_client.clone(),
            actor_id: auth_ctx.actor_id.clone(),
            credential: local_credential,
        };

        // Get verifying key for key_id from AisKeyCache (local hit or fetch from signaling)
        let verifying_key = match auth_ctx
            .ais_key_cache
            .get_or_fetch(credential.key_id, &fetcher)
            .await
        {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    key_id = credential.key_id,
                    error = ?e,
                    "WS credential verification failed: unable to get signing key"
                );
                return None;
            }
        };

        // Ed25519 signature verification
        let sig_result =
            credential.signature[..]
                .try_into()
                .ok()
                .and_then(|sig_bytes: [u8; 64]| {
                    let signature = Signature::from_bytes(&sig_bytes);
                    verifying_key
                        .verify(&credential.claims[..], &signature)
                        .ok()
                });
        if sig_result.is_none() {
            tracing::warn!(
                key_id = credential.key_id,
                "WS AIdCredential Ed25519 verification failed"
            );
            return None;
        }

        // Decode IdentityClaims
        let claims = match IdentityClaims::decode(&credential.claims[..]) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(key_id = credential.key_id, error = ?e, "WS IdentityClaims proto decode failed");
                return None;
            }
        };

        // Check expires_at
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if claims.expires_at <= now {
            tracing::warn!(
                key_id = credential.key_id,
                expires_at = claims.expires_at,
                "WS AIdCredential has expired"
            );
            return None;
        }

        // Verify claims.actor_id matches X-Actr-Source-ID (prevent identity claim mismatch)
        match ActrId::decode(source_id_bytes) {
            Ok(source_actor_id) => {
                let source_repr = source_actor_id.to_string_repr();
                if claims.actor_id != source_repr {
                    tracing::warn!(
                        claimed = %claims.actor_id,
                        source_id = %source_repr,
                        "WS credential actor_id does not match X-Actr-Source-ID, rejecting connection"
                    );
                    return None;
                }
                tracing::info!(
                    actor_id = %claims.actor_id,
                    "WS inbound connection identity verification passed"
                );
            }
            Err(e) => {
                tracing::warn!(error = ?e, "WS X-Actr-Source-ID decode failed, rejecting connection");
                return None;
            }
        }

        Some(())
    }

    /// Spawn receive tasks for a single WebSocket connection
    ///
    /// Reads `PayloadType::RpcReliable`, `RpcSignal`, `StreamReliable`,
    /// `StreamLatencyFirst` -- four lanes total, spawning an independent task for each.
    fn spawn_connection_tasks(
        conn: WebSocketConnection,
        source_id: Vec<u8>,
        pending_requests: PendingRequestsMap,
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

    /// Start the connection accept loop (called by ActrNode, can only be called once)
    ///
    /// Internally takes `conn_rx` and moves it into a background task for lock-free new connection reception.
    /// If `auth_ctx` is configured, performs credential verification for each inbound connection;
    /// drops connections on verification failure, calls `spawn_connection_tasks` on success.
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

                // Credential verification (if auth_ctx is configured)
                if let Some(ref ctx) = auth_ctx {
                    match credential_opt {
                        Some(ref credential) => {
                            if Self::verify_credential(credential, &source_id, ctx)
                                .await
                                .is_none()
                            {
                                tracing::warn!(
                                    "WS inbound connection credential verification failed, dropping connection"
                                );
                                continue; // drop connection, wait for next one
                            }
                        }
                        None => {
                            tracing::warn!(
                                "WS inbound connection missing X-Actr-Credential, rejecting connection (auth_ctx configured)"
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
                    tracing::error!(
                        "WS auth_ctx not configured, rejecting connection (configuration error)"
                    );
                }
            }

            tracing::info!("🔌 WebSocketGate receive loop exited");
        });

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::{ActrIdExt, ActrType, IdentityClaims, Realm};
    use actr_runtime_mailbox::{MailboxStats, MessageRecord, StorageResult};
    use async_trait::async_trait;
    use ed25519_dalek::{Signer, SigningKey};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    // ─── helpers ─────────────────────────────────────────────────────────────

    fn test_actor_id(serial: u64) -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number: serial,
            r#type: ActrType {
                manufacturer: "test".to_string(),
                name: "node".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    /// Generate reproducible Ed25519 key pair from a fixed seed
    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// Construct a complete AIdCredential that passes verification
    fn make_valid_credential(
        sk: &SigningKey,
        actor_id: &ActrId,
        expires_at: u64,
        key_id: u32,
    ) -> AIdCredential {
        let claims = IdentityClaims {
            actor_id: actor_id.to_string_repr(),
            expires_at,
            realm_id: actor_id.realm.realm_id,
        };
        let claims_bytes = actr_protocol::prost::Message::encode_to_vec(&claims);
        let signature = sk.sign(&claims_bytes);
        AIdCredential {
            key_id,
            claims: claims_bytes.into(),
            signature: signature.to_bytes().to_vec().into(),
        }
    }

    fn future_ts() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600
    }

    fn past_ts() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 1
    }

    // --- mock SignalingClient (same as ais_key_cache tests, inlined to avoid module dependency) ---

    struct NullSignaling;

    #[async_trait]
    impl crate::wire::SignalingClient for NullSignaling {
        async fn connect(&self) -> crate::transport::error::NetworkResult<()> {
            Ok(())
        }
        async fn disconnect(&self) -> crate::transport::error::NetworkResult<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            false
        }
        fn get_stats(&self) -> crate::wire::webrtc::SignalingStats {
            Default::default()
        }
        fn subscribe_events(
            &self,
        ) -> tokio::sync::broadcast::Receiver<crate::wire::webrtc::SignalingEvent> {
            tokio::sync::broadcast::channel(1).1
        }
        async fn set_actor_id(&self, _: ActrId) {}
        async fn set_credential_state(&self, _: crate::lifecycle::CredentialState) {}
        async fn clear_identity(&self) {}
        async fn send_register_request(
            &self,
            _: actr_protocol::RegisterRequest,
        ) -> crate::transport::error::NetworkResult<actr_protocol::RegisterResponse> {
            unimplemented!()
        }
        async fn send_unregister_request(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: Option<String>,
        ) -> crate::transport::error::NetworkResult<actr_protocol::UnregisterResponse> {
            unimplemented!()
        }
        async fn send_heartbeat(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: actr_protocol::ServiceAvailabilityState,
            _: f32,
            _: f32,
        ) -> crate::transport::error::NetworkResult<actr_protocol::Pong> {
            unimplemented!()
        }
        async fn send_route_candidates_request(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: actr_protocol::RouteCandidatesRequest,
        ) -> crate::transport::error::NetworkResult<actr_protocol::RouteCandidatesResponse>
        {
            unimplemented!()
        }
        async fn send_credential_update_request(
            &self,
            _: ActrId,
            _: AIdCredential,
        ) -> crate::transport::error::NetworkResult<actr_protocol::RegisterResponse> {
            unimplemented!()
        }
        async fn send_envelope(
            &self,
            _: actr_protocol::SignalingEnvelope,
        ) -> crate::transport::error::NetworkResult<()> {
            unimplemented!()
        }
        async fn receive_envelope(
            &self,
        ) -> crate::transport::error::NetworkResult<Option<actr_protocol::SignalingEnvelope>>
        {
            unimplemented!()
        }
        async fn get_signing_key(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: u32,
        ) -> crate::transport::error::NetworkResult<(u32, Vec<u8>)> {
            Err(crate::transport::error::NetworkError::ConnectionError(
                "should not be called".into(),
            ))
        }
    }

    /// Construct WsAuthContext with pre-seeded public key (key_id already in cache, no signaling needed)
    async fn make_auth_ctx(sk: &SigningKey, key_id: u32, local_actor: ActrId) -> WsAuthContext {
        let pubkey_bytes = sk.verifying_key().as_bytes().to_vec();
        let cache = AisKeyCache::new();
        cache.seed(key_id, &pubkey_bytes).await.unwrap();

        let local_credential = AIdCredential {
            key_id,
            claims: bytes::Bytes::new(),
            signature: bytes::Bytes::from(vec![0u8; 64]),
        };
        let cred_state = crate::lifecycle::CredentialState::new(local_credential, None, None);

        WsAuthContext {
            ais_key_cache: cache,
            actor_id: local_actor,
            credential_state: cred_state,
            signaling_client: Arc::new(NullSignaling),
        }
    }

    // ─── verify_credential ────────────────────────────────────────────────────

    /// Happy path: valid credential + matching actor_id -> Some(())
    #[tokio::test]
    async fn verify_credential_valid_returns_some() {
        let sk = signing_key(1);
        let actor = test_actor_id(100);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let credential = make_valid_credential(&sk, &actor, future_ts(), 1);
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        let result = WebSocketGate::verify_credential(&credential, &source_bytes, &ctx).await;
        assert!(result.is_some(), "valid credential should pass");
    }

    /// Signature flipped 1 bit -> verification fails -> None
    #[tokio::test]
    async fn verify_credential_tampered_signature_returns_none() {
        let sk = signing_key(2);
        let actor = test_actor_id(101);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let mut credential = make_valid_credential(&sk, &actor, future_ts(), 1);

        let mut sig = credential.signature.to_vec();
        sig[0] ^= 0xFF;
        credential.signature = sig.into();

        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);
        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    /// Signature less than 64 bytes -> try_into fails -> None
    #[tokio::test]
    async fn verify_credential_short_signature_returns_none() {
        let sk = signing_key(3);
        let actor = test_actor_id(102);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let mut credential = make_valid_credential(&sk, &actor, future_ts(), 1);

        credential.signature = bytes::Bytes::from(vec![0u8; 32]); // too short

        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);
        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    /// expires_at in the past -> expired -> None
    #[tokio::test]
    async fn verify_credential_expired_returns_none() {
        let sk = signing_key(4);
        let actor = test_actor_id(103);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let credential = make_valid_credential(&sk, &actor, past_ts(), 1);
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none(),
            "expired credential should be rejected"
        );
    }

    /// claims.actor_id does not match ActrId from X-Actr-Source-ID -> None (prevent identity spoofing)
    #[tokio::test]
    async fn verify_credential_actor_id_mismatch_returns_none() {
        let sk = signing_key(5);
        let claimed_actor = test_actor_id(200); // Credential claims actor 200.
        let actual_source = test_actor_id(201); // Actual connecting peer is 201.
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;

        // The credential is signed for `claimed_actor(200)`, but the source ID is `actual_source(201)`.
        let credential = make_valid_credential(&sk, &claimed_actor, future_ts(), 1);
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actual_source);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none(),
            "actor_id mismatch should be rejected"
        );
    }

    /// Invalid protobuf bytes in `IdentityClaims` should fail decoding and return `None`.
    #[tokio::test]
    async fn verify_credential_invalid_claims_proto_returns_none() {
        let sk = signing_key(6);
        let actor = test_actor_id(104);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;

        // Build garbage claim bytes and sign them. The signature is valid, but the claims cannot be decoded.
        let garbage = b"\xFF\xFF\xFF\xFF\xFF";
        let signature = sk.sign(garbage);
        let credential = AIdCredential {
            key_id: 1,
            claims: bytes::Bytes::from(garbage.to_vec()),
            signature: signature.to_bytes().to_vec().into(),
        };
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    /// Invalid protobuf bytes in `source_id_bytes` should fail `ActrId` decoding and return `None`.
    #[tokio::test]
    async fn verify_credential_invalid_source_id_returns_none() {
        let sk = signing_key(7);
        let actor = test_actor_id(105);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let credential = make_valid_credential(&sk, &actor, future_ts(), 1);

        let bad_source_id = b"\xFF\xFF\xFF\xFF"; // Invalid protobuf.

        assert!(
            WebSocketGate::verify_credential(&credential, bad_source_id, &ctx)
                .await
                .is_none()
        );
    }

    /// When `key_id` is missing from cache and signaling returns an error, return `None`.
    #[tokio::test]
    async fn verify_credential_unknown_key_id_returns_none() {
        let sk = signing_key(8);
        let actor = test_actor_id(106);
        // `key_id=99` is not in cache, so `NullSignaling` returns an error.
        let cache = AisKeyCache::new();
        let local_credential = AIdCredential {
            key_id: 1,
            claims: bytes::Bytes::new(),
            signature: bytes::Bytes::from(vec![0u8; 64]),
        };
        let ctx = WsAuthContext {
            ais_key_cache: cache,
            actor_id: test_actor_id(999),
            credential_state: crate::lifecycle::CredentialState::new(local_credential, None, None),
            signaling_client: Arc::new(NullSignaling),
        };

        let credential = make_valid_credential(&sk, &actor, future_ts(), 99); // `key_id=99` does not exist.
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    // ─── handle_envelope routing logic ───────────────────────────────────────

    struct CapturingMailbox {
        enqueue_count: AtomicUsize,
        last_priority: std::sync::Mutex<Option<MessagePriority>>,
    }

    impl CapturingMailbox {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                enqueue_count: AtomicUsize::new(0),
                last_priority: std::sync::Mutex::new(None),
            })
        }
    }

    #[async_trait]
    impl Mailbox for CapturingMailbox {
        async fn enqueue(
            &self,
            _from: Vec<u8>,
            _payload: Vec<u8>,
            priority: MessagePriority,
        ) -> StorageResult<Uuid> {
            self.enqueue_count.fetch_add(1, Ordering::SeqCst);
            *self.last_priority.lock().unwrap() = Some(priority);
            Ok(Uuid::new_v4())
        }
        async fn dequeue(&self) -> StorageResult<Vec<MessageRecord>> {
            Ok(vec![])
        }
        async fn ack(&self, _: Uuid) -> StorageResult<()> {
            Ok(())
        }
        async fn status(&self) -> StorageResult<MailboxStats> {
            Ok(MailboxStats {
                queued_messages: 0,
                inflight_messages: 0,
                queued_by_priority: Default::default(),
            })
        }
    }

    fn make_rpc_envelope(request_id: &str) -> RpcEnvelope {
        RpcEnvelope {
            request_id: request_id.to_string(),
            route_key: "test".to_string(),
            payload: Some(bytes::Bytes::from("hello")),
            error: None,
            timeout_ms: 5000,
            ..Default::default()
        }
    }

    type PendingReplies =
        Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>;

    fn empty_pending() -> PendingReplies {
        Arc::new(RwLock::new(HashMap::new()))
    }

    /// RPC request with no pending entry should go to the mailbox with normal priority.
    #[tokio::test]
    async fn handle_envelope_request_goes_to_mailbox_with_normal_priority() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let envelope = make_rpc_envelope("req-1");
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![1u8, 2, 3],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending,
            mailbox.clone(),
        )
        .await;

        assert_eq!(mailbox.enqueue_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *mailbox.last_priority.lock().unwrap(),
            Some(MessagePriority::Normal)
        );
    }

    /// `RpcSignal` should go to the mailbox with high priority.
    #[tokio::test]
    async fn handle_envelope_rpc_signal_uses_high_priority() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let envelope = make_rpc_envelope("sig-1");
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcSignal,
            pending,
            mailbox.clone(),
        )
        .await;

        assert_eq!(mailbox.enqueue_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *mailbox.last_priority.lock().unwrap(),
            Some(MessagePriority::High)
        );
    }

    /// RPC response with a matching pending request should wake the waiter and not enter the mailbox.
    #[tokio::test]
    async fn handle_envelope_response_resolves_pending_not_mailbox() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(1);

        let (tx, rx) = oneshot::channel();
        pending
            .write()
            .await
            .insert("req-2".to_string(), (actor, tx));

        let mut envelope = make_rpc_envelope("req-2");
        envelope.payload = Some(bytes::Bytes::from("response-payload"));
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending.clone(),
            mailbox.clone(),
        )
        .await;

        assert_eq!(
            mailbox.enqueue_count.load(Ordering::SeqCst),
            0,
            "response must not go to mailbox"
        );
        let result = rx.await.expect("oneshot must be resolved");
        assert!(result.is_ok(), "response payload should resolve Ok");
    }

    /// A response carrying both payload and error should send `Err(DecodeFailure)` to the waiter.
    #[tokio::test]
    async fn handle_envelope_response_both_payload_and_error_gives_decode_failure() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(2);
        let (tx, rx) = oneshot::channel();
        pending
            .write()
            .await
            .insert("req-3".to_string(), (actor, tx));

        let mut envelope = make_rpc_envelope("req-3");
        envelope.payload = Some(bytes::Bytes::from("x"));
        envelope.error = Some(actr_protocol::ErrorResponse {
            code: 500,
            message: "err".to_string(),
        });
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending,
            mailbox.clone(),
        )
        .await;

        let result = rx.await.unwrap();
        assert!(
            matches!(result, Err(actr_protocol::ActrError::DecodeFailure(_))),
            "both payload+error should produce DecodeFailure: {result:?}"
        );
    }

    /// A response carrying only `error` and no payload should send `Err(Unavailable)` to the waiter.
    #[tokio::test]
    async fn handle_envelope_response_error_only_gives_unavailable() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(3);
        let (tx, rx) = oneshot::channel();
        pending
            .write()
            .await
            .insert("req-4".to_string(), (actor, tx));

        let mut envelope = make_rpc_envelope("req-4");
        envelope.payload = None;
        envelope.error = Some(actr_protocol::ErrorResponse {
            code: 503,
            message: "unavailable".to_string(),
        });
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending,
            mailbox.clone(),
        )
        .await;

        let result = rx.await.unwrap();
        assert!(
            matches!(result, Err(actr_protocol::ActrError::Unavailable(_))),
            "error-only response should produce Unavailable: {result:?}"
        );
        assert_eq!(mailbox.enqueue_count.load(Ordering::SeqCst), 0);
    }

    /// After response handling, the matching pending entry should be removed.
    #[tokio::test]
    async fn handle_envelope_response_removes_pending_entry() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(4);
        let (tx, _rx) = oneshot::channel::<actr_protocol::ActorResult<Bytes>>();
        pending
            .write()
            .await
            .insert("req-5".to_string(), (actor, tx));

        let envelope = make_rpc_envelope("req-5");
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending.clone(),
            mailbox,
        )
        .await;

        assert!(
            !pending.read().await.contains_key("req-5"),
            "pending entry must be removed after response"
        );
    }
}
