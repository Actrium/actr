//! OutprocOutGate - Outproc transport adapter (outbound)
//!
//! # Responsibilities
//! - Wrap OutprocTransportManager (Protobuf serialization)
//! - Used for cross-process communication (WebRTC + WebSocket)
//! - Maintain pending_requests (Request/Response matching)
//! - Block new requests to peers being cleaned up (closing_peers)

use crate::transport::connection_event::{ConnectionEvent, ConnectionState};
use crate::transport::{Dest, OutprocTransportManager};
use crate::wire::webrtc::coordinator::{
    NETWORK_RECOVERY_TIMEOUT, NetworkRecoveryStatus, WebRtcCoordinator,
};
use actr_framework::{Bytes, MediaSample};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrId, PayloadType, ProtocolError, RpcEnvelope};
use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::sync::Arc;
#[cfg(feature = "test-utils")]
use std::time::Instant;
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

    /// Peers in the network/WebRTC recovery window. The stored session id keeps
    /// late events from older sessions from unblocking a newer recovery.
    recovering_peers: Arc<RwLock<HashMap<ActrId, NetworkRecoveryStatus>>>,
}

impl OutprocOutGate {
    fn event_kind(event: &ConnectionEvent) -> &'static str {
        match event {
            ConnectionEvent::StateChanged { .. } => "StateChanged",
            ConnectionEvent::DataChannelClosed { .. } => "DataChannelClosed",
            ConnectionEvent::DataChannelOpened { .. } => "DataChannelOpened",
            ConnectionEvent::ConnectionClosed { .. } => "ConnectionClosed",
            ConnectionEvent::IceRestartStarted { .. } => "IceRestartStarted",
            ConnectionEvent::IceRestartCompleted { .. } => "IceRestartCompleted",
            ConnectionEvent::NewOfferReceived { .. } => "NewOfferReceived",
            ConnectionEvent::NewRoleAssignment { .. } => "NewRoleAssignment",
        }
    }

    fn remember_recovering_peer(
        recovering: &mut HashMap<ActrId, NetworkRecoveryStatus>,
        peer_id: &ActrId,
        session_id: u64,
        reason: &str,
    ) {
        match recovering.entry(peer_id.clone()) {
            Entry::Occupied(entry) if entry.get().session_id == session_id => {
                tracing::debug!(
                    peer_id = ?peer_id,
                    session_id = session_id,
                    elapsed_ms = entry.get().elapsed_ms(),
                    recovery_reason = entry.get().reason.as_str(),
                    "🚧 Peer already blocked for recovery"
                );
            }
            Entry::Occupied(mut entry) => {
                entry.insert(NetworkRecoveryStatus::new(session_id, reason));
            }
            Entry::Vacant(entry) => {
                entry.insert(NetworkRecoveryStatus::new(session_id, reason));
            }
        }
    }

    fn recovering_error(target: &ActrId, status: &NetworkRecoveryStatus) -> ProtocolError {
        ProtocolError::TransportError(format!(
            "Connection recovering: peer={:?}, session_id={}, reason={}, elapsed_ms={}, timeout_ms={}",
            target,
            status.session_id,
            status.reason.as_str(),
            status.elapsed_ms(),
            NETWORK_RECOVERY_TIMEOUT.as_millis()
        ))
    }

    fn recovery_timeout_error(target: &ActrId, status: &NetworkRecoveryStatus) -> ProtocolError {
        ProtocolError::TransportError(format!(
            "Connection recovery timeout: peer={:?}, session_id={}, reason={}, elapsed_ms={}, timeout_ms={}",
            target,
            status.session_id,
            status.reason.as_str(),
            status.elapsed_ms(),
            NETWORK_RECOVERY_TIMEOUT.as_millis()
        ))
    }

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
        let recovering_peers = Arc::new(RwLock::new(HashMap::new()));
        let pending_requests = Arc::new(RwLock::new(HashMap::new()));

        // Start event listener if coordinator is available
        // This is the ONLY event subscriber - it triggers top-down cleanup
        if let Some(ref coordinator) = webrtc_coordinator {
            Self::spawn_event_listener(
                coordinator.subscribe_events(),
                Arc::clone(coordinator),
                Arc::clone(&pending_requests),
                Arc::clone(&closing_peers),
                Arc::clone(&recovering_peers),
                Arc::clone(&transport_manager),
            );
        }

        Self {
            transport_manager,
            pending_requests,
            webrtc_coordinator,
            closing_peers,
            recovering_peers,
        }
    }

    /// Spawn event listener task to handle connection events
    ///
    /// This is the **ONLY** event subscriber in the cleanup chain.
    /// It triggers top-down cleanup by calling transport_manager.close_transport().
    fn spawn_event_listener(
        mut event_rx: broadcast::Receiver<ConnectionEvent>,
        webrtc_coordinator: Arc<WebRtcCoordinator>,
        pending_requests: Arc<
            RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>,
        >,
        closing_peers: Arc<RwLock<HashSet<ActrId>>>,
        recovering_peers: Arc<RwLock<HashMap<ActrId, NetworkRecoveryStatus>>>,
        transport_manager: Arc<OutprocTransportManager>,
    ) {
        tokio::spawn(async move {
            loop {
                let event = match event_rx.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            "⚠️ OutprocOutGate event listener lagged by {} events, continuing",
                            n
                        );
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!(
                            "🔌 OutprocOutGate event listener stopped (channel closed)"
                        );
                        break;
                    }
                };
                tracing::debug!(
                    "🔄 OutprocOutGate received connection event: event_kind={}, event_session_id={:?}, event={:?}",
                    Self::event_kind(&event),
                    event.session_id(),
                    event
                );
                match &event {
                    // Block new requests when connection enters Disconnected/Failed state
                    ConnectionEvent::StateChanged {
                        peer_id,
                        session_id,
                        state: ConnectionState::Disconnected | ConnectionState::Failed,
                        ..
                    } => {
                        if !webrtc_coordinator
                            .is_active_session(peer_id, *session_id)
                            .await
                        {
                            tracing::debug!(
                                "⏭️ Ignoring stale recovery state event: peer={}, event_session_id={}",
                                peer_id,
                                session_id
                            );
                            continue;
                        }

                        {
                            let mut recovering = recovering_peers.write().await;
                            Self::remember_recovering_peer(
                                &mut recovering,
                                peer_id,
                                *session_id,
                                "peer state Disconnected/Failed",
                            );
                        }
                        closing_peers.write().await.insert(peer_id.clone());
                        tracing::debug!(
                            "🚫 Blocking new requests to peer {} (state: Disconnected/Failed)",
                            peer_id,
                        );
                    }

                    ConnectionEvent::IceRestartStarted {
                        peer_id,
                        session_id,
                    } => {
                        {
                            let mut recovering = recovering_peers.write().await;
                            Self::remember_recovering_peer(
                                &mut recovering,
                                peer_id,
                                *session_id,
                                "ice/network recovery started",
                            );
                        }
                        tracing::debug!("🚧 Peer {} entered ICE/network recovery", peer_id);
                    }

                    ConnectionEvent::StateChanged {
                        peer_id,
                        session_id,
                        state: ConnectionState::Connected,
                        ..
                    }
                    | ConnectionEvent::DataChannelOpened {
                        peer_id,
                        session_id,
                        payload_type: PayloadType::RpcReliable,
                        ..
                    }
                    | ConnectionEvent::IceRestartCompleted {
                        peer_id,
                        session_id,
                        success: true,
                        ..
                    } => {
                        let should_clear = {
                            let recovering = recovering_peers.read().await;
                            recovering
                                .get(peer_id)
                                .map(|status| status.session_id == *session_id)
                                .unwrap_or(true)
                        };

                        if should_clear {
                            recovering_peers.write().await.remove(peer_id);
                            closing_peers.write().await.remove(peer_id);
                            tracing::debug!("✅ Peer {} is sendable again", peer_id);
                        } else {
                            tracing::debug!(
                                "⏭️ Ignoring sendable event for stale session: peer={}, event_session_id={}",
                                peer_id,
                                session_id
                            );
                        }
                    }

                    ConnectionEvent::IceRestartCompleted {
                        peer_id,
                        session_id,
                        success: false,
                        ..
                    } => {
                        let should_clear = {
                            let recovering = recovering_peers.read().await;
                            recovering
                                .get(peer_id)
                                .map(|status| status.session_id == *session_id)
                                .unwrap_or(true)
                        };
                        if should_clear {
                            let mut recovering = recovering_peers.write().await;
                            Self::remember_recovering_peer(
                                &mut recovering,
                                peer_id,
                                *session_id,
                                "ice restart failed",
                            );
                            closing_peers.write().await.insert(peer_id.clone());
                        }
                        tracing::debug!(
                            "🚫 Peer {} ICE restart failed; keeping sends blocked",
                            peer_id
                        );
                    }

                    // Clean pending requests and trigger downstream cleanup when connection is fully closed
                    ConnectionEvent::StateChanged {
                        peer_id,
                        session_id,
                        state: ConnectionState::Closed,
                        ..
                    }
                    | ConnectionEvent::ConnectionClosed {
                        peer_id,
                        session_id,
                    } => {
                        let event_kind = Self::event_kind(&event);
                        let event_session_id = *session_id;

                        // Mark peer as closing (release lock immediately to avoid deadlock)
                        {
                            closing_peers.write().await.insert(peer_id.clone());
                        } // Lock released here

                        let pending_before = {
                            let pending = pending_requests.read().await;
                            pending
                                .values()
                                .filter(|(target, _)| target == peer_id)
                                .count()
                        };

                        // 1. Trigger downstream cleanup (OutprocTransportManager → DestTransport → WirePool)
                        // Note: We don't hold closing_peers lock here to avoid deadlock when
                        // close_transport needs to acquire its own locks or when multiple
                        // connections are closing simultaneously during shutdown.
                        let dest = Dest::actor(peer_id.clone());
                        let should_cleanup_pending = match transport_manager
                            .close_transport_if_webrtc_session(&dest, peer_id, event_session_id)
                            .await
                        {
                            Ok(true) => {
                                tracing::info!(
                                    "✅ Successfully closed transport chain for peer {} (session_id={})",
                                    peer_id,
                                    event_session_id
                                );
                                true
                            }
                            Ok(false) => {
                                tracing::warn!(
                                    "⏭️ Skipped transport cleanup for peer {} because event session is stale or transport changed (event_kind={}, event_session_id={})",
                                    peer_id,
                                    event_kind,
                                    event_session_id
                                );
                                false
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "⚠️ Failed to close transport for peer {} (session_id={}): {}",
                                    peer_id,
                                    event_session_id,
                                    e
                                );
                                true
                            }
                        };

                        // A Closed/ConnectionClosed event means this recovery session cannot
                        // become sendable. Clear the matching guard even when transport cleanup
                        // is skipped because the transport was already removed by a broader
                        // cleanup path.
                        {
                            let mut recovering = recovering_peers.write().await;
                            let should_remove = recovering
                                .get(peer_id)
                                .map(|status| status.session_id == event_session_id)
                                .unwrap_or(false);
                            if should_remove {
                                recovering.remove(peer_id);
                                tracing::debug!(
                                    "✅ Cleared recovery guard for closed peer {} (session_id={})",
                                    peer_id,
                                    event_session_id
                                );
                            }
                        }
                        webrtc_coordinator
                            .expire_peer_recovery(peer_id, event_session_id, "connection closed")
                            .await;

                        if !should_cleanup_pending {
                            closing_peers.write().await.remove(peer_id);
                            continue;
                        }

                        let transport_exists_after_cleanup =
                            transport_manager.has_dest(&dest).await;

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

                        tracing::debug!(
                            "🧹 OutprocOutGate cleanup result: peer_id={}, event_kind={}, event_session_id={:?}, pending_before={}, pending_cleaned={}, transport_exists_after_cleanup={}",
                            peer_id,
                            event_kind,
                            Some(event_session_id),
                            pending_before,
                            cleaned_count,
                            transport_exists_after_cleanup
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

                        closing_peers.write().await.remove(peer_id);
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

    async fn clear_local_recovery_guard(&self, target: &ActrId, session_id: u64) {
        let mut recovering = self.recovering_peers.write().await;
        let should_remove = recovering
            .get(target)
            .map(|status| status.session_id == session_id)
            .unwrap_or(false);
        if should_remove {
            recovering.remove(target);
        }
        self.closing_peers.write().await.remove(target);
    }

    async fn handle_recovery_timeout(
        &self,
        target: &ActrId,
        dest: &Dest,
        status: &NetworkRecoveryStatus,
        source: &str,
    ) -> ProtocolError {
        tracing::warn!(
            peer = ?target,
            session_id = status.session_id,
            elapsed_ms = status.elapsed_ms(),
            recovery_reason = status.reason.as_str(),
            source = source,
            "⏱️ Connection recovery timed out; closing stale transport"
        );

        self.clear_local_recovery_guard(target, status.session_id)
            .await;

        if let Some(coordinator) = &self.webrtc_coordinator {
            coordinator
                .close_recovering_peer(target, status.session_id, "send preflight recovery timeout")
                .await;
        }

        if let Err(e) = self.transport_manager.close_transport(dest).await {
            tracing::warn!(
                peer = ?target,
                session_id = status.session_id,
                "⚠️ Failed to close transport after recovery timeout: {}",
                e
            );
        }

        Self::recovery_timeout_error(target, status)
    }

    #[cfg(feature = "test-utils")]
    pub async fn force_recovery_started_at_for_test(
        &self,
        target: &ActrId,
        started_at: Instant,
    ) -> bool {
        let mut recovering = self.recovering_peers.write().await;
        if let Some(status) = recovering.get_mut(target) {
            status.started_at = started_at;
            true
        } else {
            false
        }
    }

    async fn preflight_send(&self, target: &ActrId, dest: &Dest) -> Result<(), ProtocolError> {
        if let Some(coordinator) = &self.webrtc_coordinator {
            if let Some(status) = coordinator.peer_recovery_status(target).await {
                if status.is_timed_out() {
                    return Err(self
                        .handle_recovery_timeout(target, dest, &status, "coordinator")
                        .await);
                }
                return Err(Self::recovering_error(target, &status));
            }
        }

        let local_recovery = {
            let recovering = self.recovering_peers.read().await;
            recovering.get(target).cloned()
        };
        if let Some(status) = local_recovery {
            if status.is_timed_out() {
                return Err(self
                    .handle_recovery_timeout(target, dest, &status, "outproc gate")
                    .await);
            }
            return Err(Self::recovering_error(target, &status));
        }

        if self.closing_peers.read().await.contains(target)
            || self.transport_manager.is_closing(dest).await
        {
            return Err(ProtocolError::TransportError(format!(
                "Connection recovering: peer={:?}, reason=transport closing",
                target,
            )));
        }

        Ok(())
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

        // 1. Convert ActrId to Dest and fail fast during recovery before
        // registering pending_requests.
        let dest = Self::actr_id_to_dest(target);
        self.preflight_send(target, &dest).await?;

        // 2. Create oneshot channel for receiving response
        let (response_tx, response_rx) = oneshot::channel();

        // 3. Register pending request with target ActorId
        {
            let mut pending = self.pending_requests.write().await;
            pending.insert(envelope.request_id.clone(), (target.clone(), response_tx));
        }

        // 4. Serialize RpcEnvelope
        let data = Self::serialize_envelope(&envelope);

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
        self.preflight_send(target, &dest).await?;
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
