//! OutprocOutGate - Outproc transport adapter (outbound)
//!
//! # Responsibilities
//! - Wrap OutprocTransportManager (Protobuf serialization)
//! - Used for cross-process communication (WebRTC + WebSocket)
//! - Maintain pending_requests (Request/Response matching)

use crate::transport::{Dest, OutprocTransportManager};
use actr_framework::{Bytes, MediaSample};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrId, PayloadType, ProtocolError, RpcEnvelope};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, oneshot};

/// OutprocOutGate - Outproc transport adapter (outbound)
///
/// # Features
/// - Protobuf serialization: serialize RpcEnvelope to byte stream
/// - Defaults to PayloadType::RpcReliable for RPC messages
/// - Maintain pending_requests for Request/Response matching
/// - Support MediaTrack sending via WebRTC
pub struct OutprocOutGate {
    /// OutprocTransportManager instance
    transport_manager: Arc<OutprocTransportManager>,

    /// Pending requests: request_id → oneshot::Sender<Bytes>
    /// Pending requests (can receive success or error)
    pending_requests:
        Arc<RwLock<HashMap<String, oneshot::Sender<actr_protocol::ActorResult<Bytes>>>>>,

    /// WebRTC coordinator (optional, for MediaTrack support)
    webrtc_coordinator: Option<Arc<crate::wire::webrtc::WebRtcCoordinator>>,
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
        Self {
            transport_manager,
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            webrtc_coordinator,
        }
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

        if let Some(tx) = pending.remove(request_id) {
            // Wake up waiting request with result (success or error)
            let _ = tx.send(result);
            tracing::debug!("✅ Completed request: {}", request_id);
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
    ) -> Arc<RwLock<HashMap<String, oneshot::Sender<actr_protocol::ActorResult<Bytes>>>>> {
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
    /// Send request and wait for response (bidirectional communication)
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        tracing::debug!(
            "📤 OutprocGate::send_request to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // 1. Create oneshot channel for receiving response
        let (response_tx, response_rx) = oneshot::channel();

        // 2. Register pending request
        {
            let mut pending = self.pending_requests.write().await;
            pending.insert(envelope.request_id.clone(), response_tx);
        }

        // 3. Serialize RpcEnvelope
        let data = Self::serialize_envelope(&envelope);

        // 4. Convert ActrId to Dest
        let dest = Self::actr_id_to_dest(target);

        // 5. Send message (defaults to PayloadType::RpcReliable)
        match self
            .transport_manager
            .send(&dest, PayloadType::RpcReliable, &data)
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
            Ok(Err(_)) => {
                // oneshot channel closed (shouldn't happen)
                Err(ProtocolError::TransportError(
                    "Response channel closed".to_string(),
                ))
            }
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

    /// Send one-way message (no response expected)
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        tracing::debug!("📤 OutprocGate::send_message to {:?}", target);

        // 1. Serialize RpcEnvelope
        let data = Self::serialize_envelope(&envelope);

        // 2. Convert ActrId to Dest
        let dest = Self::actr_id_to_dest(target);

        // 3. Send message (defaults to PayloadType::RpcReliable)
        self.transport_manager
            .send(&dest, PayloadType::RpcReliable, &data)
            .await
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;

        tracing::debug!("✅ Sent message to {:?}", target);
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

        // Convert ActrId to Dest
        let dest = Self::actr_id_to_dest(target);

        // Send via transport manager
        self.transport_manager
            .send(&dest, payload_type, &data)
            .await
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;

        tracing::debug!("✅ Sent DataStream to {:?}", target);
        Ok(())
    }
}

impl Drop for OutprocOutGate {
    fn drop(&mut self) {
        tracing::debug!("🗑️  OutprocGate dropped");
    }
}
