//! OutprocOutGate - Outproc transport adapter (outbound)
//!
//! # Responsibilities
//! - Wrap OutprocTransportManager (Protobuf serialization)
//! - Used for cross-process communication (WebRTC + WebSocket)
//! - Maintain pending_requests (Request/Response matching)
//! - Block new requests to peers being cleaned up (closing_peers)

use crate::transport::connection_event::{ConnectionEvent, ConnectionState};
use crate::transport::{Dest, OutprocTransportManager};
use actr_framework::{Bytes, MediaSample};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrId, PayloadType, ProtocolError, RpcEnvelope};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, oneshot};

/// OutprocOutGate - Outproc transport adapter (outbound)
///
/// # Features
/// - Protobuf serialization: serialize RpcEnvelope to byte stream
/// - Defaults to PayloadType::RpcReliable for RPC messages
/// - Maintain pending_requests for Request/Response matching
/// - Support MediaTrack sending via WebRTC
/// - Block new requests to peers being cleaned up (closing_peers)
pub struct OutprocOutGate {
    /// OutprocTransportManager instance
    transport_manager: Arc<OutprocTransportManager>,

    /// Pending requests: request_id → (target_actor_id, oneshot::Sender<Bytes>)
    /// Stores both the target ActorId and response sender for efficient cleanup by peer
    pending_requests:
        Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>,

    /// WebRTC coordinator (optional, for MediaTrack support)
    webrtc_coordinator: Option<Arc<crate::wire::webrtc::WebRtcCoordinator>>,

    #[allow(unused)]
    /// todo: Peers currently being cleaned up (block new requests) ,closed requests will be cleaned up in event listener
    closing_peers: Arc<RwLock<HashSet<ActrId>>>,
}

impl OutprocOutGate {
    /// Create new OutprocOutGate
    ///
    /// # Arguments
    /// - `transport_manager`: OutprocTransportManager instance
    /// - `webrtc_coordinator`: Optional WebRTC coordinator for MediaTrack support
    pub fn new(
        transport_manager: Arc<OutprocTransportManager>,
        webrtc_coordinator: Option<Arc<crate::wire::webrtc::WebRtcCoordinator>>,
    ) -> Self {
        let closing_peers = Arc::new(RwLock::new(HashSet::new()));
        let pending_requests = Arc::new(RwLock::new(HashMap::new()));

        // Start event listener if coordinator is available
        // This is the ONLY event subscriber - it triggers top-down cleanup
        if let Some(ref coordinator) = webrtc_coordinator {
            Self::spawn_event_listener(
                coordinator.subscribe_events(),
                Arc::clone(&pending_requests),
                Arc::clone(&closing_peers),
                Arc::clone(&transport_manager),
            );
        }

        Self {
            transport_manager,
            pending_requests,
            webrtc_coordinator,
            closing_peers,
        }
    }

    /// Spawn event listener task to handle connection events
    ///
    /// This is the **ONLY** event subscriber in the cleanup chain.
    /// It triggers top-down cleanup by calling transport_manager.close_transport().
    fn spawn_event_listener(
        mut event_rx: broadcast::Receiver<ConnectionEvent>,
        pending_requests: Arc<
            RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>,
        >,
        closing_peers: Arc<RwLock<HashSet<ActrId>>>,
        transport_manager: Arc<OutprocTransportManager>,
    ) {
        tokio::spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                tracing::debug!("🔄 OutprocOutGate received connection event: {:?}", event);
                match &event {
                    // Block new requests when connection enters Disconnected/Failed state
                    ConnectionEvent::StateChanged {
                        peer_id,
                        state: ConnectionState::Disconnected | ConnectionState::Failed,
                    } => {
                        closing_peers.write().await.insert(peer_id.clone());
                        tracing::debug!(
                            "🚫 Blocking new requests to peer {} (state: Disconnected/Failed)",
                            peer_id,
                        );
                    }

                    // Clean pending requests and trigger downstream cleanup when connection is fully closed
                    ConnectionEvent::StateChanged {
                        peer_id,
                        state: ConnectionState::Closed,
                    }
                    | ConnectionEvent::ConnectionClosed { peer_id } => {
                        // Mark peer as closing (release lock immediately to avoid deadlock)
                        {
                            closing_peers.write().await.insert(peer_id.clone());
                        } // Lock released here

                        // 1. Trigger downstream cleanup (OutprocTransportManager → DestTransport → WirePool)
                        // Note: We don't hold closing_peers lock here to avoid deadlock when
                        // close_transport needs to acquire its own locks or when multiple
                        // connections are closing simultaneously during shutdown.
                        let dest = Dest::actor(peer_id.clone());
                        match transport_manager.close_transport(&dest).await {
                            Ok(_) => {
                                tracing::info!(
                                    "✅ Successfully closed transport chain for peer {}",
                                    peer_id
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "⚠️ Failed to close transport for peer {}: {}",
                                    peer_id,
                                    e
                                );
                            }
                        }

                        // 2. Clean pending requests for this peer
                        let mut pending = pending_requests.write().await;

                        // Collect request_ids that belong to this peer
                        let keys_to_remove: Vec<_> = pending
                            .iter()
                            .filter_map(|(req_id, (target, _))| {
                                if target == peer_id {
                                    Some(req_id.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        let cleaned_count = keys_to_remove.len();

                        tracing::info!(
                            "🧹 Cleaned {} pending requests for peer {}",
                            cleaned_count,
                            peer_id
                        );

                        // Remove and send error to all pending requests for this peer
                        for key in keys_to_remove {
                            if let Some((_, tx)) = pending.remove(&key) {
                                let _ = tx.send(Err(ProtocolError::TransportError(
                                    "Connection closed".to_string(),
                                )));
                            }
                        }
                        drop(pending); // Release lock before calling downstream

                        // Unblock after cleanup completes
                        closing_peers.write().await.remove(peer_id);
                    }

                    // Unblock peer when ICE restart succeeds
                    ConnectionEvent::IceRestartCompleted {
                        peer_id,
                        success: true,
                    } => {
                        closing_peers.write().await.remove(peer_id);
                        tracing::debug!(
                            "✅ Unblocked peer {} after successful ICE restart",
                            peer_id
                        );
                    }

                    _ => {} // Ignore other events
                }
            }
        });
    }

    /// Handle response message (called by MessageDispatcher)
    ///
    /// # Arguments
    /// - `request_id`: Request ID
    /// - `result`: Response data (Ok) or error (Err)
    ///
    /// # Returns
    /// - `Ok(true)`: Successfully woke up waiting request
    /// - `Ok(false)`: No corresponding pending request found
    pub async fn handle_response(
        &self,
        request_id: &str,
        result: actr_protocol::ActorResult<Bytes>,
    ) -> ActorResult<bool> {
        let mut pending = self.pending_requests.write().await;

        if let Some((target, tx)) = pending.remove(request_id) {
            // Wake up waiting request with result (success or error)
            let _ = tx.send(result);
            tracing::debug!("✅ Completed request: {} (target: {})", request_id, target);
            Ok(true)
        } else {
            tracing::warn!("⚠️  No pending request for: {}", request_id);
            Ok(false)
        }
    }

    /// Get pending requests count (for monitoring)
    pub async fn pending_count(&self) -> usize {
        self.pending_requests.read().await.len()
    }

    /// Get pending_requests reference (for WebRtcGate to share)
    pub fn get_pending_requests(
        &self,
    ) -> Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>
    {
        self.pending_requests.clone()
    }

    /// Convert ActrId to Dest
    fn actr_id_to_dest(actor_id: &ActrId) -> Dest {
        Dest::actor(actor_id.clone())
    }

    /// Serialize RpcEnvelope to bytes
    fn serialize_envelope(envelope: &RpcEnvelope) -> Vec<u8> {
        envelope.encode_to_vec()
    }
}

impl OutprocOutGate {
    /// Send request and wait for response (with specified PayloadType).
    ///
    /// This is primarily used by language bindings / non-generic RPC paths.
    pub async fn send_request_with_type(
        &self,
        target: &ActrId,
        payload_type: PayloadType,
        envelope: RpcEnvelope,
    ) -> ActorResult<Bytes> {
        tracing::debug!(
            "📤 OutprocGate::send_request_with_type to {:?}, payload_type={:?}, request_id={}",
            target,
            payload_type,
            envelope.request_id
        );

        // 1. Create oneshot channel for receiving response
        let (response_tx, response_rx) = oneshot::channel();

        // 2. Register pending request with target ActorId
        {
            let mut pending = self.pending_requests.write().await;
            pending.insert(envelope.request_id.clone(), (target.clone(), response_tx));
        }

        // 3. Serialize RpcEnvelope
        let data = Self::serialize_envelope(&envelope);

        // 4. Convert ActrId to Dest
        let dest = Self::actr_id_to_dest(target);

        // 5. Send message using the specified payload_type
        match self
            .transport_manager
            .send(&dest, payload_type, &data)
            .await
        {
            Ok(_) => {
                tracing::debug!("✅ Sent request to {:?}", target);
            }
            Err(e) => {
                // Send failed, remove pending request
                self.pending_requests
                    .write()
                    .await
                    .remove(&envelope.request_id);
                return Err(ProtocolError::TransportError(e.to_string()));
            }
        }

        // 6. Wait for response (timeout from envelope.timeout_ms)
        let timeout = std::time::Duration::from_millis(envelope.timeout_ms as u64);

        match tokio::time::timeout(timeout, response_rx).await {
            Ok(Ok(result)) => {
                // result is ActorResult<Bytes>, propagate it
                tracing::debug!("✅ Received response for request: {}", envelope.request_id);
                result
            }
            Ok(Err(_)) => Err(ProtocolError::TransportError(
                "Response channel closed".to_string(),
            )),
            Err(_) => {
                // Timeout
                self.pending_requests
                    .write()
                    .await
                    .remove(&envelope.request_id);
                Err(ProtocolError::TransportError(format!(
                    "Request timeout: {}ms",
                    envelope.timeout_ms
                )))
            }
        }
    }

    /// Send request and wait for response (bidirectional communication)
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "OutprocOutGate.send_request")
    )]
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        self.send_request_with_type(target, PayloadType::RpcReliable, envelope)
            .await
    }

    /// Send one-way message (no response expected)
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "OutprocOutGate.send_message", fields(target = %target))
    )]
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        tracing::debug!("📤 OutprocGate::send_message to {:?}", target);

        // // Check if target is being cleaned up
        // if self.closing_peers.read().await.contains(target) {
        //     return Err(ProtocolError::TransportError(format!(
        //         "Connection to {} is closing",
        //         target.to_string_repr()
        //     )));
        // }

        self.send_message_with_type(target, PayloadType::RpcReliable, envelope)
            .await
    }

    /// Send one-way message with specified PayloadType.
    pub async fn send_message_with_type(
        &self,
        target: &ActrId,
        payload_type: PayloadType,
        envelope: RpcEnvelope,
    ) -> ActorResult<()> {
        tracing::debug!(
            "📤 OutprocGate::send_message_with_type to {:?}, payload_type={:?}",
            target,
            payload_type
        );

        let data = Self::serialize_envelope(&envelope);
        let dest = Self::actr_id_to_dest(target);
        self.transport_manager
            .send(&dest, payload_type, &data)
            .await
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;
        Ok(())
    }

    /// Send media sample via WebRTC native track
    ///
    /// # Parameters
    /// - `target`: Target Actor ID
    /// - `track_id`: Media track identifier
    /// - `sample`: Media sample data
    ///
    /// # Implementation Note
    /// Delegates to WebRtcCoordinator which manages WebRTC Tracks
    pub async fn send_media_sample(
        &self,
        target: &ActrId,
        track_id: &str,
        sample: MediaSample,
    ) -> ActorResult<()> {
        tracing::debug!(
            "📤 OutprocGate::send_media_sample to {:?}, track_id={}",
            target,
            track_id
        );

        // Check if WebRTC coordinator is available
        let coordinator = self.webrtc_coordinator.as_ref().ok_or_else(|| {
            ProtocolError::Actr(actr_protocol::ActrError::NotImplemented {
                feature: "MediaTrack requires WebRTC coordinator".to_string(),
            })
        })?;

        // Delegate to WebRtcCoordinator
        coordinator
            .send_media_sample(target, track_id, sample)
            .await
            .map_err(|e| ProtocolError::TransportError(format!("WebRTC send failed: {e}")))?;

        tracing::debug!("✅ Sent media sample to {:?}", target);
        Ok(())
    }

    /// Send DataStream (Fast Path)
    ///
    /// # Parameters
    /// - `target`: Target Actor ID
    /// - `payload_type`: PayloadType (StreamReliable or StreamLatencyFirst)
    /// - `data`: Serialized DataStream bytes
    ///
    /// # Implementation Note
    /// Sends via OutprocTransportManager using WebRTC DataChannel or WebSocket
    pub async fn send_data_stream(
        &self,
        target: &ActrId,
        payload_type: PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        tracing::debug!(
            "📤 OutprocGate::send_data_stream to {:?}, payload_type={:?}, size={} bytes",
            target,
            payload_type,
            data.len()
        );

        // // Check if target is being cleaned up
        // if self.closing_peers.read().await.contains(target) {
        //     return Err(ProtocolError::TransportError(format!(
        //         "Connection to {} is closing",
        //         target.to_string_repr()
        //     )));
        // }

        // Convert ActrId to Dest
        let dest = Self::actr_id_to_dest(target);

        // Send via transport manager
        let result = self
            .transport_manager
            .send(&dest, payload_type, &data)
            .await
            .map_err(|e| ProtocolError::TransportError(e.to_string()));

        result
    }
}

impl Drop for OutprocOutGate {
    fn drop(&mut self) {
        tracing::debug!("🗑️  OutprocGate dropped");
    }
}
