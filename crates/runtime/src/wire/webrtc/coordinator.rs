// WebRTC Signaling Coordinator - Coordinates WebRTC P2P connection establishment

#[allow(dead_code)]
fn is_ipv4_candidate_allowed(cand: &str) -> bool {
    // Only filter out IPv6 candidates (link-local and other IPv6 addresses)
    // Allow all IPv4 candidates (private and public IPs)
    if cand.contains("fe80::") || cand.contains(" udp6 ") || cand.contains("::") {
        return false;
    }

    // Accept all IPv4 candidates by default
    // This includes: loopback (127.x), private (10.x, 172.x, 192.168.x), and public IPs
    true
}

// Responsibilities:
// - Listen to WebRTC signaling messages from SignalingClient
// - Handle Offer/Answer/ICE candidate exchanges
// - Establish and manage RTCPeerConnection instances
// - Create and cache WebRtcConnection instances
// - Aggregate messages from all peers

use super::connection::WebRtcConnection;
use super::negotiator::WebRtcNegotiator;
use super::{SignalingClient, WebRtcConfig};
use crate::error::{RuntimeError, RuntimeResult};
use crate::inbound::MediaFrameRegistry;
use actr_framework::Bytes;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrRelay, PayloadType, SignalingEnvelope, actr_relay,
    session_description::Type as SdpType, signaling_envelope,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_local::TrackLocalWriter;

/// Type alias for message receiver (from all peers)
type MessageRx = Arc<Mutex<mpsc::UnboundedReceiver<(Vec<u8>, Bytes, PayloadType)>>>;

/// Peer connection state
struct PeerState {
    /// RTCPeerConnection (for receiving ICE candidates)
    peer_connection: Arc<RTCPeerConnection>,

    /// WebRtcConnection (for business message transmission)
    webrtc_conn: WebRtcConnection,

    /// Connection ready notification (for initiate_connection to wait)
    ready_tx: Option<oneshot::Sender<()>>,
}

/// WebRTC signaling coordinator
pub struct WebRtcCoordinator {
    /// Local Actor ID
    local_id: ActrId,

    /// Local credentials
    credential: AIdCredential,

    /// SignalingClient (for sending ICE/SDP)
    signaling_client: Arc<dyn SignalingClient>,

    /// WebRTC negotiator
    negotiator: WebRtcNegotiator,

    /// Peer state mapping (ActrId → PeerState)
    peers: Arc<RwLock<HashMap<ActrId, PeerState>>>,

    /// Pending ICE candidates (received before remote description is set)
    /// ActrId → Vec<candidate_string>
    pending_candidates: Arc<RwLock<HashMap<ActrId, Vec<String>>>>,

    /// Message receive channel (aggregated from all peers)
    /// (from: ActrId bytes, data: Bytes)
    /// Format: (sender_id_bytes, message_data, payload_type)
    message_rx: MessageRx,
    message_tx: mpsc::UnboundedSender<(Vec<u8>, Bytes, PayloadType)>,

    /// MediaTrack callback registry (for WebRTC native media streams)
    media_frame_registry: Arc<MediaFrameRegistry>,
}

impl WebRtcCoordinator {
    /// Create new coordinator
    pub fn new(
        local_id: ActrId,
        credential: AIdCredential,
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_config: WebRtcConfig,
        media_frame_registry: Arc<MediaFrameRegistry>,
    ) -> Self {
        let (message_tx, message_rx) = mpsc::unbounded_channel();

        Self {
            local_id,
            credential,
            signaling_client,
            negotiator: WebRtcNegotiator::new(webrtc_config),
            peers: Arc::new(RwLock::new(HashMap::new())),
            pending_candidates: Arc::new(RwLock::new(HashMap::new())),
            message_rx: Arc::new(Mutex::new(message_rx)),
            message_tx,
            media_frame_registry,
        }
    }

    /// Start signaling coordinator (listen for ActrRelay messages)
    ///
    /// This method starts a background task that continuously listens for messages from SignalingClient
    /// and handles WebRTC-related signaling (Offer/Answer/ICE)
    pub async fn start(self: Arc<Self>) -> RuntimeResult<()> {
        tracing::info!("🚀 WebRtcCoordinator starting signaling loop");

        let coordinator = self.clone();
        tokio::spawn(async move {
            loop {
                // 1. Receive message from SignalingClient
                match coordinator.signaling_client.receive_envelope().await {
                    Ok(Some(envelope)) => {
                        // 2. Decode SignalingEnvelope
                        if let Some(signaling_envelope::Flow::ActrRelay(relay)) = envelope.flow {
                            let source = relay.source;

                            // 3. Dispatch based on payload type
                            match relay.payload {
                                Some(actr_relay::Payload::SessionDescription(sd)) => {
                                    match sd.r#type() {
                                        SdpType::Offer => {
                                            tracing::info!(
                                                "📥 Received Offer from {:?}",
                                                source.serial_number
                                            );
                                            if let Err(e) =
                                                coordinator.handle_offer(&source, sd.sdp).await
                                            {
                                                tracing::error!("❌ Failed to handle Offer: {}", e);
                                            }
                                        }
                                        SdpType::Answer => {
                                            tracing::info!(
                                                "📥 Received Answer from {:?}",
                                                source.serial_number
                                            );
                                            if let Err(e) =
                                                coordinator.handle_answer(&source, sd.sdp).await
                                            {
                                                tracing::error!(
                                                    "❌ Failed to handle Answer: {}",
                                                    e
                                                );
                                            }
                                        }
                                        SdpType::RenegotiationOffer => {
                                            tracing::warn!(
                                                "⚠️ Received RenegotiationOffer, not supported yet"
                                            );
                                        }
                                    }
                                }
                                Some(actr_relay::Payload::IceCandidate(ice)) => {
                                    tracing::trace!(
                                        "📥 Received ICE Candidate from {:?}",
                                        source.serial_number
                                    );
                                    if let Err(e) = coordinator
                                        .handle_ice_candidate(&source, ice.candidate)
                                        .await
                                    {
                                        tracing::error!("❌ Failed to handle ICE Candidate: {}", e);
                                    }
                                }
                                None => {
                                    tracing::warn!("⚠️ ActrRelay missing payload");
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::info!(
                            "🔌 SignalingClient connection closed, exiting signaling loop"
                        );
                        break;
                    }
                    Err(e) => {
                        tracing::error!("❌ Signaling receive error: {}", e);
                        // Continue loop, don't exit (may be temporary error)
                    }
                }
            }

            tracing::info!("🛑 WebRtcCoordinator signaling loop exited");
        });

        Ok(())
    }

    /// Send ActrRelay message (internal helper method)
    async fn send_actr_relay(
        &self,
        target: &ActrId,
        payload: actr_relay::Payload,
    ) -> RuntimeResult<()> {
        let relay = ActrRelay {
            source: self.local_id.clone(),
            credential: self.credential.clone(),
            target: target.clone(),
            payload: Some(payload),
        };

        let flow = signaling_envelope::Flow::ActrRelay(relay);

        let envelope = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: uuid::Uuid::new_v4().to_string(),
            reply_for: None,
            timestamp: prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            },
            flow: Some(flow),
        };

        self.signaling_client
            .send_envelope(envelope)
            .await
            .map_err(|e| RuntimeError::Unavailable {
                message: format!("Signaling server unavailable: {e}"),
                target: None,
            })?;

        Ok(())
    }

    /// Initiate connection (create Offer)
    ///
    /// Acts as the initiator, sending a WebRTC connection request to the target peer
    pub async fn initiate_connection(
        self: &Arc<Self>,
        target: &ActrId,
    ) -> RuntimeResult<oneshot::Receiver<()>> {
        tracing::info!(
            "🚀 Initiating P2P connection to serial={}",
            target.serial_number
        );

        // 1. Create RTCPeerConnection
        let peer_connection = self.negotiator.create_peer_connection().await?;
        let peer_connection_arc = Arc::new(peer_connection);

        // 2. Create WebRtcConnection (shares Arc<RTCPeerConnection>)
        let webrtc_conn = WebRtcConnection::new(Arc::clone(&peer_connection_arc));

        // 3. Pre-create negotiated DataChannel for Reliable to trigger ICE gathering
        // Both sides will create the same channel ID, so this works for both offerer and answerer
        let _reliable_lane = webrtc_conn
            .get_lane(actr_protocol::PayloadType::RpcReliable)
            .await?;
        tracing::debug!("Pre-created Reliable DataChannel for ICE gathering");

        // 3.5. Pre-create media tracks for sending (MUST be done before creating Offer)
        // Create a default video track for demo purposes
        let _video_track = webrtc_conn
            .add_media_track("video-track-1".to_string(), "VP8", "video")
            .await?;
        tracing::debug!("Pre-created video MediaTrack: video-track-1");

        // 4. Register on_track callback for receiving MediaTrack (WebRTC native media)
        let media_registry = Arc::clone(&self.media_frame_registry);
        let sender_id = target.clone();
        peer_connection_arc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let media_registry = Arc::clone(&media_registry);
            let sender_id = sender_id.clone();
            Box::pin(async move {
                let track_id = track.id();
                tracing::info!(
                    "📹 Received MediaTrack: track_id={}, sender={}",
                    track_id,
                    sender_id.serial_number
                );

                // Spawn task to read RTP packets from track
                tokio::spawn(async move {
                    loop {
                        // Read RTP packet from track
                        match track.read_rtp().await {
                            Ok((rtp_packet, _attributes)) => {
                                // Extract payload and timestamp
                                let payload_data = rtp_packet.payload.clone();
                                let timestamp = rtp_packet.header.timestamp;

                                // TODO: Extract codec from track (for now use placeholder)
                                let codec = "unknown".to_string();

                                // Create MediaSample
                                let sample = actr_framework::MediaSample {
                                    data: payload_data,
                                    timestamp,
                                    codec,
                                    media_type: actr_framework::MediaType::Video, // TODO: detect from track
                                };

                                // Dispatch to registered callback
                                media_registry
                                    .dispatch(&track_id, sample, sender_id.clone())
                                    .await;
                            }
                            Err(e) => {
                                tracing::error!(
                                    "❌ Failed to read RTP from track {}: {}",
                                    track_id,
                                    e
                                );
                                break;
                            }
                        }
                    }
                    tracing::info!("🛑 MediaTrack reader task exited for track_id={}", track_id);
                });
            })
        }));

        // 5. Set ICE candidate callback (local ICE candidate collection)
        let coordinator = Arc::downgrade(self);
        let target_id = target.clone();
        peer_connection_arc.on_ice_candidate(Box::new(
            move |candidate: Option<RTCIceCandidate>| {
                let coordinator = coordinator.clone();
                let target_id = target_id.clone();
                Box::pin(async move {
                    if let Some(cand) = candidate {
                        if let Some(coord) = coordinator.upgrade() {
                            // Convert RTCIceCandidate to JSON string (webrtc crate's standard method)
                            let candidate_json = match cand.to_json() {
                                Ok(json) => json.candidate,
                                Err(e) => {
                                    tracing::error!("❌ ICE Candidate serialization failed: {}", e);
                                    return;
                                }
                            };

                            let ice_candidate = actr_protocol::IceCandidate {
                                candidate: candidate_json,
                                sdp_mid: None,
                                sdp_mline_index: None,
                                username_fragment: None,
                            };

                            let payload = actr_relay::Payload::IceCandidate(ice_candidate);
                            if let Err(e) = coord.send_actr_relay(&target_id, payload).await {
                                tracing::error!("❌ Failed to send ICE Candidate: {}", e);
                            } else {
                                tracing::trace!("✅ Sent ICE Candidate");
                            }
                        }
                    }
                })
            },
        ));

        // 5. Create Offer
        let offer_sdp = self.negotiator.create_offer(&peer_connection_arc).await?;

        // 6. Create ready notification channel
        let (ready_tx, ready_rx) = oneshot::channel();

        // 7. Store peer state BEFORE sending Offer (prevent race condition)
        {
            let mut peers = self.peers.write().await;
            tracing::info!(
                "🔧 [STORE] Inserting peer: realm={}, serial={}, type={}:{}, current peers={}",
                target.realm.realm_id,
                target.serial_number,
                target.r#type.manufacturer,
                target.r#type.name,
                peers.len()
            );
            peers.insert(
                target.clone(),
                PeerState {
                    peer_connection: peer_connection_arc.clone(),
                    webrtc_conn: webrtc_conn.clone(),
                    ready_tx: Some(ready_tx),
                },
            );
            tracing::info!("✅ [STORE] Peer inserted, new total={}", peers.len());
        }

        // 8. Send Offer via signaling server (AFTER storing peer state)
        let session_desc = actr_protocol::SessionDescription {
            r#type: SdpType::Offer as i32,
            sdp: offer_sdp,
        };
        let payload = actr_relay::Payload::SessionDescription(session_desc);
        self.send_actr_relay(target, payload).await?;

        tracing::info!("✅ Sent Offer to serial={}", target.serial_number);

        // 9. Start receive loop (receive and aggregate messages from this peer)
        self.start_peer_receive_loop(target.clone(), webrtc_conn)
            .await;

        Ok(ready_rx)
    }

    /// Handle received Offer (passive side)
    ///
    /// Called when receiving a connection request from another peer.
    /// Supports both initial negotiation and renegotiation.
    async fn handle_offer(self: &Arc<Self>, from: &ActrId, offer_sdp: String) -> RuntimeResult<()> {
        // Check if this is a renegotiation (peer state already exists)
        let is_renegotiation = self.peers.read().await.contains_key(from);

        if is_renegotiation {
            tracing::info!(
                "🔄 Handling renegotiation Offer from serial={}",
                from.serial_number
            );
            return self.handle_renegotiation_offer(from, offer_sdp).await;
        }

        tracing::info!(
            "📥 Handling initial Offer from serial={}",
            from.serial_number
        );

        // 1. Create RTCPeerConnection
        let peer_connection = self.negotiator.create_peer_connection().await?;
        let peer_connection_arc = Arc::new(peer_connection);

        // 2. Create WebRtcConnection (shares Arc<RTCPeerConnection>)
        let webrtc_conn = WebRtcConnection::new(Arc::clone(&peer_connection_arc));

        // 3. Pre-create negotiated DataChannel for Reliable to trigger ICE gathering
        // Both sides will create the same channel ID, so this works for both offerer and answerer
        let _reliable_lane = webrtc_conn
            .get_lane(actr_protocol::PayloadType::RpcReliable)
            .await?;
        tracing::debug!("Pre-created Reliable DataChannel for ICE gathering (answerer)");

        // 3.5. Pre-create media tracks for sending (MUST be done before creating Answer)
        // Create a default video track for demo purposes
        let _video_track = webrtc_conn
            .add_media_track("video-track-1".to_string(), "VP8", "video")
            .await?;
        tracing::debug!("Pre-created video MediaTrack: video-track-1 (answerer)");

        // 4. Register on_track callback for receiving MediaTrack (WebRTC native media)
        let media_registry = Arc::clone(&self.media_frame_registry);
        let sender_id = from.clone();
        peer_connection_arc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let media_registry = Arc::clone(&media_registry);
            let sender_id = sender_id.clone();
            Box::pin(async move {
                let track_id = track.id();
                tracing::info!(
                    "📹 Received MediaTrack: track_id={}, sender={}",
                    track_id,
                    sender_id.serial_number
                );

                // Spawn task to read RTP packets from track
                tokio::spawn(async move {
                    loop {
                        // Read RTP packet from track
                        match track.read_rtp().await {
                            Ok((rtp_packet, _attributes)) => {
                                // Extract payload and timestamp
                                let payload_data = rtp_packet.payload.clone();
                                let timestamp = rtp_packet.header.timestamp;

                                // TODO: Extract codec from track (for now use placeholder)
                                let codec = "unknown".to_string();

                                // Create MediaSample
                                let sample = actr_framework::MediaSample {
                                    data: payload_data,
                                    timestamp,
                                    codec,
                                    media_type: actr_framework::MediaType::Video, // TODO: detect from track
                                };

                                // Dispatch to registered callback
                                media_registry
                                    .dispatch(&track_id, sample, sender_id.clone())
                                    .await;
                            }
                            Err(e) => {
                                tracing::error!(
                                    "❌ Failed to read RTP from track {}: {}",
                                    track_id,
                                    e
                                );
                                break;
                            }
                        }
                    }
                    tracing::info!("🛑 MediaTrack reader task exited for track_id={}", track_id);
                });
            })
        }));

        // 5. Set ICE candidate callback (local ICE candidate collection)
        let coordinator = Arc::downgrade(self);
        let target_id = from.clone();
        peer_connection_arc.on_ice_candidate(Box::new(
            move |candidate: Option<RTCIceCandidate>| {
                let coordinator = coordinator.clone();
                let target_id = target_id.clone();
                Box::pin(async move {
                    if let Some(cand) = candidate {
                        if let Some(coord) = coordinator.upgrade() {
                            // Convert RTCIceCandidate to JSON string (webrtc crate's standard method)
                            let candidate_json = match cand.to_json() {
                                Ok(json) => json.candidate,
                                Err(e) => {
                                    tracing::error!("❌ ICE Candidate serialization failed: {}", e);
                                    return;
                                }
                            };

                            let ice_candidate = actr_protocol::IceCandidate {
                                candidate: candidate_json,
                                sdp_mid: None,
                                sdp_mline_index: None,
                                username_fragment: None,
                            };

                            let payload = actr_relay::Payload::IceCandidate(ice_candidate);
                            if let Err(e) = coord.send_actr_relay(&target_id, payload).await {
                                tracing::error!("❌ Failed to send ICE Candidate: {}", e);
                            } else {
                                tracing::trace!("✅ Sent ICE Candidate");
                            }
                        }
                    }
                })
            },
        ));

        // 5. Create Answer
        let answer_sdp = self
            .negotiator
            .create_answer(&peer_connection_arc, offer_sdp)
            .await?;

        // 6. Store peer state BEFORE sending Answer (prevent race condition)
        {
            let mut peers = self.peers.write().await;
            peers.insert(
                from.clone(),
                PeerState {
                    peer_connection: peer_connection_arc.clone(),
                    webrtc_conn: webrtc_conn.clone(),
                    ready_tx: None,
                },
            );
        }

        // 7. Send Answer via signaling server (AFTER storing peer state)
        let session_desc = actr_protocol::SessionDescription {
            r#type: SdpType::Answer as i32,
            sdp: answer_sdp,
        };
        let payload = actr_relay::Payload::SessionDescription(session_desc);
        self.send_actr_relay(from, payload).await?;

        tracing::info!("✅ Sent Answer to serial={}", from.serial_number);

        // 8. Flush any buffered ICE candidates (remote description is now set)
        self.flush_pending_candidates(from, &peer_connection_arc)
            .await?;

        // 9. Start receive loop
        self.start_peer_receive_loop(from.clone(), webrtc_conn)
            .await;

        Ok(())
    }

    /// Handle received Answer (initiator side)
    ///
    /// Supports both initial negotiation and renegotiation answers.
    async fn handle_answer(
        self: &Arc<Self>,
        from: &ActrId,
        answer_sdp: String,
    ) -> RuntimeResult<()> {
        // Get corresponding PeerConnection and ready_tx
        let (peer_connection, ready_tx, is_renegotiation) = {
            let mut peers = self.peers.write().await;
            tracing::info!(
                "🔍 [LOOKUP] Searching for: realm={}, serial={}, type={}:{}, total peers={}",
                from.realm.realm_id,
                from.serial_number,
                from.r#type.manufacturer,
                from.r#type.name,
                peers.len()
            );
            for (k, _) in peers.iter() {
                tracing::info!(
                    "   📌 [LOOKUP] Stored: realm={}, serial={}, type={}:{}",
                    k.realm.realm_id,
                    k.serial_number,
                    k.r#type.manufacturer,
                    k.r#type.name
                );
            }
            let state = peers.get_mut(from).ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!("Peer not found: {}", from.serial_number))
            })?;

            let pc = state.peer_connection.clone();
            let tx = state.ready_tx.take();
            let is_reneg = tx.is_none(); // If ready_tx already taken, this is renegotiation
            (pc, tx, is_reneg)
        };

        if is_renegotiation {
            tracing::info!(
                "🔄 Handling renegotiation Answer from serial={}",
                from.serial_number
            );
        } else {
            tracing::info!(
                "📥 Handling initial Answer from serial={}",
                from.serial_number
            );
        }

        // Handle Answer (set remote SDP)
        self.negotiator
            .handle_answer(&peer_connection, answer_sdp)
            .await?;

        // Flush any buffered ICE candidates (remote description is now set)
        self.flush_pending_candidates(from, &peer_connection)
            .await?;

        tracing::info!(
            "✅ WebRTC connection negotiation completed: serial={}",
            from.serial_number
        );

        // Wait for PeerConnection to actually connect (max 5 seconds)
        let pc_clone = peer_connection.clone();
        tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            loop {
                let state = pc_clone.connection_state();
                if state == webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Connected {
                    tracing::info!("✅ PeerConnection fully connected");
                    break;
                }
                if state == webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Failed {
                    tracing::error!("❌ PeerConnection failed");
                    return;
                }
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    tracing::warn!("⚠️ PeerConnection connection timeout (5s)");
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }

            // Notify initiate_connection that connection is ready
            if let Some(tx) = ready_tx {
                let _ = tx.send(());
            }
        });

        Ok(())
    }

    /// Flush buffered ICE candidates for a peer
    ///
    /// Called after remote description is set, to add any candidates that arrived early
    async fn flush_pending_candidates(
        &self,
        peer_id: &ActrId,
        peer_connection: &RTCPeerConnection,
    ) -> RuntimeResult<()> {
        // Extract buffered candidates for this peer
        let candidates = {
            let mut pending = self.pending_candidates.write().await;
            pending.remove(peer_id)
        };

        if let Some(candidates) = candidates {
            tracing::debug!(
                "🔄 Flushing {} buffered ICE candidates for {:?}",
                candidates.len(),
                peer_id
            );

            for candidate in candidates {
                if let Err(e) = self
                    .negotiator
                    .add_ice_candidate(peer_connection, candidate)
                    .await
                {
                    tracing::warn!("⚠️ Failed to add buffered ICE candidate: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Handle received ICE Candidate
    async fn handle_ice_candidate(
        self: &Arc<Self>,
        from: &ActrId,
        candidate: String,
    ) -> RuntimeResult<()> {
        tracing::trace!("📥 Received ICE Candidate from {:?}", from);

        // DEBUG: Temporarily disable candidate filtering for local testing
        // TODO: Re-enable proper filtering for production
        // if !is_ipv4_candidate_allowed(&candidate) {
        //     tracing::debug!("🚫 Ignoring ICE candidate from {:?}: {}", from, candidate);
        //     return Ok(());
        // }

        // Try to get peer and check if remote description is set
        let peer_opt = {
            let peers = self.peers.read().await;
            peers.get(from).map(|state| state.peer_connection.clone())
        };

        match peer_opt {
            Some(peer_connection) => {
                // Check if remote description is set
                if peer_connection.remote_description().await.is_some() {
                    // Can add candidate immediately
                    self.negotiator
                        .add_ice_candidate(&peer_connection, candidate)
                        .await?;
                    tracing::trace!("✅ Added ICE Candidate from {:?}", from);
                } else {
                    // Buffer for later (remote description not yet set)
                    self.pending_candidates
                        .write()
                        .await
                        .entry(from.clone())
                        .or_insert_with(Vec::new)
                        .push(candidate);
                    tracing::debug!(
                        "🔖 Buffered ICE candidate from {:?} (remote description not yet set)",
                        from
                    );
                }
            }
            None => {
                // Buffer for when peer is created
                self.pending_candidates
                    .write()
                    .await
                    .entry(from.clone())
                    .or_insert_with(Vec::new)
                    .push(candidate);
                tracing::debug!(
                    "🔖 Buffered ICE candidate from {:?} (peer not yet created)",
                    from
                );
            }
        }

        Ok(())
    }

    /// Start peer receive loop
    ///
    /// Starts a background task for each peer to receive messages from WebRtcConnection and aggregate to a unified message_tx
    ///
    /// IMPORTANT: We need to listen to ALL PayloadTypes, not just RpcReliable:
    /// - RpcReliable, RpcSignal: for RPC messages
    /// - StreamReliable, StreamLatencyFirst: for DataStream messages
    async fn start_peer_receive_loop(&self, peer_id: ActrId, webrtc_conn: WebRtcConnection) {
        let message_tx = self.message_tx.clone();

        // Listen to all relevant PayloadTypes
        let payload_types = vec![
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
        ];

        for payload_type in payload_types {
            let message_tx_clone = message_tx.clone();
            let peer_id_clone = peer_id.clone();
            let webrtc_conn_clone = webrtc_conn.clone();

            tokio::spawn(async move {
                tracing::debug!(
                    "📡 Starting receive loop for peer {:?}, PayloadType: {:?}",
                    peer_id_clone,
                    payload_type
                );

                // Get Lane for this PayloadType
                let lane = match webrtc_conn_clone.get_lane(payload_type).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!(
                            "❌ Failed to get Lane for {:?}, PayloadType {:?}: {}",
                            peer_id_clone,
                            payload_type,
                            e
                        );
                        return;
                    }
                };

                // Continuously receive messages
                loop {
                    match lane.recv().await {
                        Ok(data) => {
                            tracing::debug!(
                                "📨 Received message from {:?} (PayloadType: {:?}): {} bytes",
                                peer_id_clone,
                                payload_type,
                                data.len()
                            );

                            // Serialize peer_id as bytes
                            let peer_id_bytes = peer_id_clone.encode_to_vec();

                            // Send to aggregation channel (include PayloadType)
                            if let Err(e) = message_tx_clone.send((peer_id_bytes, data, payload_type)) {
                                tracing::error!("❌ Message aggregation failed: {:?}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "❌ Peer {:?} message receive failed (PayloadType: {:?}): {}",
                                peer_id_clone,
                                payload_type,
                                e
                            );
                            break;
                        }
                    }
                }

                tracing::debug!(
                    "📡 Receive loop exited for peer {:?}, PayloadType: {:?}",
                    peer_id_clone,
                    payload_type
                );
            });
        }
    }

    /// Send message to specified peer
    ///
    /// If connection doesn't exist, automatically initiates WebRTC connection and waits for it to be ready
    pub async fn send_message(self: &Arc<Self>, target: &ActrId, data: &[u8]) -> RuntimeResult<()> {
        tracing::debug!("📤 Sending message to {:?}: {} bytes", target, data.len());

        // Check if connection exists
        let has_connection = {
            let peers = self.peers.read().await;
            peers.contains_key(target)
        };

        // If connection doesn't exist, initiate connection
        if !has_connection {
            tracing::info!(
                "🔗 First send to {:?}, initiating WebRTC connection",
                target.serial_number
            );

            let ready_rx = self.initiate_connection(target).await?;

            // Wait for connection to be ready (30s timeout)
            match tokio::time::timeout(std::time::Duration::from_secs(30), ready_rx).await {
                Ok(Ok(())) => {
                    tracing::info!(
                        "✅ WebRTC connection ready: serial={}",
                        target.serial_number
                    );
                }
                Ok(Err(_)) => {
                    return Err(RuntimeError::Other(anyhow::anyhow!(
                        "Connection establishment failed (channel closed)"
                    )));
                }
                Err(_) => {
                    return Err(RuntimeError::DeadlineExceeded {
                        message: "Connection establishment timeout".to_string(),
                        timeout_ms: 30000,
                    });
                }
            }
        }

        // Get corresponding WebRtcConnection
        let webrtc_conn = {
            let peers = self.peers.read().await;
            peers
                .get(target)
                .map(|state| state.webrtc_conn.clone())
                .ok_or_else(|| {
                    RuntimeError::Other(anyhow::anyhow!("Peer connection not found: {target:?}"))
                })?
        };

        // Get Reliable Lane
        let lane = webrtc_conn
            .get_lane(PayloadType::RpcReliable)
            .await
            .map_err(|e| RuntimeError::Other(anyhow::anyhow!("Failed to get Lane: {e}")))?;

        // Send message (convert to Bytes)
        lane.send(Bytes::copy_from_slice(data))
            .await
            .map_err(|e| RuntimeError::Other(anyhow::anyhow!("Failed to send message: {e}")))?;

        Ok(())
    }

    /// Receive message (aggregated from all peers)
    /// Receive message with PayloadType information
    ///
    /// Returns: Option<(sender_id_bytes, message_data, payload_type)>
    pub async fn receive_message(&self) -> RuntimeResult<Option<(Vec<u8>, Bytes, PayloadType)>> {
        let mut rx = self.message_rx.lock().await;
        Ok(rx.recv().await)
    }

    /// Create WebRTC connection (factory method)
    ///
    /// For ConnectionFactory, creates a WebRTC connection to the specified Dest.
    /// If connection already exists, returns it directly; otherwise initiates new connection and waits for it to be ready.
    ///
    /// # Arguments
    /// - `dest`: destination (must be Actor type)
    ///
    /// # Returns
    /// - `Ok(WebRtcConnection)`: ready WebRTC connection
    /// - `Err`: WebRTC only supports Actor targets, or connection establishment failed
    pub async fn create_connection(
        self: &Arc<Self>,
        dest: &crate::transport::Dest,
    ) -> RuntimeResult<WebRtcConnection> {
        // 1. Check if dest is Actor
        let target_id = dest.as_actor_id().ok_or_else(|| {
            RuntimeError::ConfigurationError(
                "WebRTC only supports Actor targets, not Shell".to_string(),
            )
        })?;

        tracing::debug!(
            "🏭 [Factory] Creating WebRTC connection to {:?}",
            target_id.serial_number
        );

        // 2. Check if connection already exists
        {
            let peers = self.peers.read().await;
            if let Some(state) = peers.get(target_id) {
                tracing::debug!(
                    "♻️ [Factory] Reusing existing WebRTC connection: {:?}",
                    target_id.serial_number
                );
                return Ok(state.webrtc_conn.clone());
            }
        }

        // 3. Initiate new connection
        tracing::info!(
            "🔨 [Factory] Initiating new WebRTC connection: {:?}",
            target_id.serial_number
        );
        let ready_rx = self.initiate_connection(target_id).await?;

        // 4. Wait for connection to be ready (30s timeout)
        tokio::time::timeout(std::time::Duration::from_secs(30), ready_rx)
            .await
            .map_err(|_| RuntimeError::DeadlineExceeded {
                message: "WebRTC connection establishment timeout".to_string(),
                timeout_ms: 30000,
            })?
            .map_err(|_| {
                RuntimeError::Other(anyhow::anyhow!(
                    "Connection establishment failed (channel closed)"
                ))
            })?;

        tracing::info!(
            "✅ [Factory] WebRTC connection ready: {:?}",
            target_id.serial_number
        );

        // 5. Get and return WebRtcConnection
        let webrtc_conn = {
            let peers = self.peers.read().await;
            peers
                .get(target_id)
                .map(|state| state.webrtc_conn.clone())
                .ok_or_else(|| {
                    RuntimeError::Other(anyhow::anyhow!(
                        "Peer not found after connection establishment"
                    ))
                })?
        };

        Ok(webrtc_conn)
    }

    /// Send media sample to target Actor via WebRTC Track
    ///
    /// # Arguments
    /// - `target`: Target Actor ID
    /// - `track_id`: Media track identifier
    /// - `sample`: Media sample to send
    ///
    /// # Returns
    /// Ok(()) if sent successfully
    pub async fn send_media_sample(
        &self,
        target: &actr_protocol::ActrId,
        track_id: &str,
        sample: actr_framework::MediaSample,
    ) -> RuntimeResult<()> {
        use webrtc::rtp::header::Header as RtpHeader;
        use webrtc::rtp::packet::Packet as RtpPacket;

        // 1. Get PeerState for target
        let peers = self.peers.read().await;
        let peer_state = peers.get(target).ok_or_else(|| {
            RuntimeError::Other(anyhow::anyhow!(
                "No connection to target: {:?}",
                target.serial_number
            ))
        })?;

        // 2. Get Track from WebRtcConnection
        let track = peer_state
            .webrtc_conn
            .get_media_track(track_id)
            .await
            .ok_or_else(|| RuntimeError::Other(anyhow::anyhow!("Track not found: {track_id}")))?;

        // 3. Get next sequence number for this track
        let sequence_number = peer_state
            .webrtc_conn
            .next_sequence_number(track_id)
            .await
            .ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!(
                    "Sequence number not found for track: {track_id}"
                ))
            })?;

        // 4. Get SSRC for this track
        let ssrc = peer_state
            .webrtc_conn
            .get_ssrc(track_id)
            .await
            .ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!("SSRC not found for track: {track_id}"))
            })?;

        // 5. Construct RTP packet from MediaSample
        let rtp_packet = RtpPacket {
            header: RtpHeader {
                version: 2,
                padding: false,
                extension: false,
                marker: true,     // Mark each sample (simplified)
                payload_type: 96, // Dynamic payload type (simplified - TODO: codec-specific)
                sequence_number,  // Per-track sequence number (wraps at 65535)
                timestamp: sample.timestamp,
                ssrc, // Unique SSRC per track (randomly generated)
                ..Default::default()
            },
            payload: sample.data,
        };

        // 6. Send RTP packet via track
        track
            .write_rtp(&rtp_packet)
            .await
            .map_err(|e| RuntimeError::Other(anyhow::anyhow!("Failed to write RTP: {e}")))?;

        tracing::debug!(
            "📤 Sent MediaSample: track_id={}, seq={}, ssrc=0x{:08x}, timestamp={}, size={}",
            track_id,
            sequence_number,
            ssrc,
            sample.timestamp,
            rtp_packet.payload.len()
        );

        Ok(())
    }

    /// Add dynamic media track and trigger SDP renegotiation
    ///
    /// # Arguments
    /// - `target`: Target Actor ID
    /// - `track_id`: Media track identifier
    /// - `codec`: Codec name (e.g., "VP8", "H264", "OPUS")
    /// - `media_type`: Media type ("video" or "audio")
    ///
    /// # Returns
    /// Ok(()) if track added and renegotiation completed successfully
    ///
    /// # Note
    /// This triggers SDP renegotiation on the existing PeerConnection.
    /// The connection remains active and existing tracks continue transmitting.
    pub async fn add_dynamic_track(
        &self,
        target: &actr_protocol::ActrId,
        track_id: String,
        codec: &str,
        media_type: &str,
    ) -> RuntimeResult<()> {
        tracing::info!(
            "🎬 Adding dynamic track: track_id={}, codec={}, type={}, target={}",
            track_id,
            codec,
            media_type,
            target.serial_number
        );

        // 1. Get existing peer state and extract needed parts
        let (webrtc_conn, peer_connection) = {
            let peers = self.peers.read().await;
            let state = peers.get(target).ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!(
                    "No connection to target: {:?}",
                    target.serial_number
                ))
            })?;
            (state.webrtc_conn.clone(), state.peer_connection.clone())
        };

        // 2. Add track to existing PeerConnection
        webrtc_conn
            .add_media_track(track_id.clone(), codec, media_type)
            .await?;

        tracing::info!("✅ Added track to PeerConnection: {}", track_id);

        // 3. Trigger SDP renegotiation
        self.renegotiate_connection(target, &peer_connection)
            .await?;

        tracing::info!("✅ Dynamic track added successfully: {}", track_id);

        Ok(())
    }

    /// Renegotiate SDP with existing peer
    ///
    /// Creates new Offer with updated track list and exchanges SDP.
    /// ICE connection remains active (no restart).
    async fn renegotiate_connection(
        &self,
        target: &actr_protocol::ActrId,
        peer_connection: &Arc<RTCPeerConnection>,
    ) -> RuntimeResult<()> {
        tracing::info!(
            "🔄 Starting SDP renegotiation with serial={}",
            target.serial_number
        );

        // 1. Create new Offer (includes all tracks: old + new)
        let offer = peer_connection.create_offer(None).await.map_err(|e| {
            RuntimeError::Other(anyhow::anyhow!(
                "Failed to create renegotiation offer: {e}"
            ))
        })?;
        let offer_sdp = offer.sdp.clone();

        // 2. Set local description
        peer_connection
            .set_local_description(offer)
            .await
            .map_err(|e| {
                RuntimeError::Other(anyhow::anyhow!("Failed to set local description: {e}"))
            })?;

        tracing::debug!(
            "📝 Created renegotiation Offer (SDP length: {})",
            offer_sdp.len()
        );

        // 3. Send Offer via signaling server
        let session_desc = actr_protocol::SessionDescription {
            r#type: SdpType::Offer as i32,
            sdp: offer_sdp,
        };
        let payload = actr_relay::Payload::SessionDescription(session_desc);
        self.send_actr_relay(target, payload).await?;

        tracing::info!(
            "✅ Sent renegotiation Offer to serial={}",
            target.serial_number
        );

        // 4. Answer will be handled by existing handle_answer() method
        // Note: We don't wait for Answer here to avoid blocking.
        // The renegotiation completes asynchronously when Answer arrives.

        Ok(())
    }

    /// Handle renegotiation Offer (existing connection)
    ///
    /// Called when receiving an Offer on an already-established connection.
    /// This happens when the remote peer adds/removes tracks dynamically.
    async fn handle_renegotiation_offer(
        &self,
        from: &ActrId,
        offer_sdp: String,
    ) -> RuntimeResult<()> {
        tracing::info!(
            "🔄 Processing renegotiation Offer from serial={}",
            from.serial_number
        );

        // 1. Get existing peer connection
        let peer_connection = {
            let peers = self.peers.read().await;
            let state = peers.get(from).ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!("Peer state not found for renegotiation"))
            })?;
            state.peer_connection.clone()
        };

        // 2. Set remote description (new Offer)
        let offer =
            webrtc::peer_connection::sdp::session_description::RTCSessionDescription::offer(
                offer_sdp,
            )
            .map_err(|e| {
                RuntimeError::Other(anyhow::anyhow!(
                    "Failed to parse renegotiation offer: {e}"
                ))
            })?;
        peer_connection
            .set_remote_description(offer)
            .await
            .map_err(|e| {
                RuntimeError::Other(anyhow::anyhow!("Failed to set remote description: {e}"))
            })?;

        tracing::debug!("✅ Set remote description (renegotiation Offer)");

        // 3. Create Answer
        let answer = peer_connection.create_answer(None).await.map_err(|e| {
            RuntimeError::Other(anyhow::anyhow!(
                "Failed to create renegotiation answer: {e}"
            ))
        })?;
        let answer_sdp = answer.sdp.clone();

        // 4. Set local description
        peer_connection
            .set_local_description(answer)
            .await
            .map_err(|e| {
                RuntimeError::Other(anyhow::anyhow!("Failed to set local description: {e}"))
            })?;

        tracing::debug!(
            "✅ Created renegotiation Answer (SDP length: {})",
            answer_sdp.len()
        );

        // 5. Send Answer via signaling server
        let session_desc = actr_protocol::SessionDescription {
            r#type: SdpType::Answer as i32,
            sdp: answer_sdp,
        };
        let payload = actr_relay::Payload::SessionDescription(session_desc);
        self.send_actr_relay(from, payload).await?;

        tracing::info!(
            "✅ Sent renegotiation Answer to serial={}",
            from.serial_number
        );

        // Note: on_track callback will automatically trigger for new remote tracks
        // No need to manually handle track additions here

        Ok(())
    }
}
