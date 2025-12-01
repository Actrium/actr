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
#[cfg(feature = "opentelemetry")]
use super::trace;
use super::{SignalingClient, WebRtcConfig};
use crate::error::{RuntimeError, RuntimeResult};
use crate::inbound::MediaFrameRegistry;
use actr_framework::Bytes;
use actr_protocol::ActrIdExt;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrRelay, PayloadType, RoleAssignment, RoleNegotiation,
    SignalingEnvelope, actr_relay, session_description::Type as SdpType, signaling_envelope,
};
use std::collections::HashMap;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::Instrument;
use tracing::debug;
#[cfg(feature = "opentelemetry")]
use tracing_opentelemetry::OpenTelemetrySpanExt;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::peer_connection::{RTCPeerConnection, peer_connection_state::RTCPeerConnectionState};
use webrtc::track::track_local::TrackLocalWriter;

const ICE_RESTART_MAX_RETRIES: u32 = 5;
const ICE_RESTART_TIMEOUT: Duration = Duration::from_secs(5);
const ICE_RESTART_INITIAL_BACKOFF_MS: u64 = 1000;
const ICE_RESTART_MAX_BACKOFF_MS: u64 = 30000;
const ROLE_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Initial connection retry constants
const INITIAL_CONNECTION_MAX_RETRIES: u32 = 3;
const INITIAL_CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
const INITIAL_CONNECTION_INITIAL_BACKOFF_MS: u64 = 500;
const INITIAL_CONNECTION_MAX_BACKOFF_MS: u64 = 5000;

/// Simple exponential backoff iterator for retries
#[derive(Debug)]
struct ExponentialBackoff {
    current_retries: u32,
    max_retries: Option<u32>,
    initial_delay: Duration,
    max_delay: Duration,
}

impl ExponentialBackoff {
    pub fn new(initial_delay: Duration, max_delay: Duration, max_retries: Option<u32>) -> Self {
        Self {
            current_retries: 0,
            max_retries,
            initial_delay,
            max_delay,
        }
    }
}

impl Iterator for ExponentialBackoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        let delay = self.initial_delay;

        if let Some(max_retries) = self.max_retries {
            self.current_retries += 1;
            if self.current_retries > max_retries {
                return None;
            }
        }

        self.initial_delay = (self.initial_delay * 2).min(self.max_delay);
        Some(delay)
    }
}

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

    /// Whether we are the offerer for the current session (affects ICE restart handling)
    is_offerer: bool,

    /// Whether ICE restart is in progress (controls buffering and retries)
    ice_restart_inflight: bool,

    /// Restart attempts counter (resets on success)
    ice_restart_attempts: u32,
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

    /// Pending role negotiation responders keyed by target ActrId
    pending_role: Arc<Mutex<HashMap<ActrId, oneshot::Sender<bool>>>>,

    /// Pending ready notifiers for answerer path
    pending_ready: Arc<Mutex<HashMap<ActrId, oneshot::Sender<()>>>>,

    /// Pending ready receivers for proactive offerer path (avoid skipping readiness)
    pending_ready_wait: Arc<Mutex<HashMap<ActrId, oneshot::Receiver<()>>>>,

    /// Cached role decisions keyed by peer ActrId (true = offerer)
    negotiated_role: Arc<Mutex<HashMap<ActrId, bool>>>,

    /// Track in-flight ICE restart tasks per peer to avoid duplicate restarts
    in_flight_restarts: Arc<Mutex<HashMap<ActrId, JoinHandle<()>>>>,
    /// Root tracing contexts for connection initiation (ActrId → Context)
    #[cfg(feature = "opentelemetry")]
    root_context_map: Arc<RwLock<HashMap<ActrId, opentelemetry::Context>>>,
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
            pending_role: Arc::new(Mutex::new(HashMap::new())),
            pending_ready: Arc::new(Mutex::new(HashMap::new())),
            pending_ready_wait: Arc::new(Mutex::new(HashMap::new())),
            negotiated_role: Arc::new(Mutex::new(HashMap::new())),
            in_flight_restarts: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "opentelemetry")]
            root_context_map: Arc::new(RwLock::new(HashMap::new())),
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
                        let span = tracing::info_span!(
                            "signaling.handle_envelope",
                            envelope_id = envelope.envelope_id,
                            reply_for = ?envelope.reply_for
                        );
                        #[cfg(feature = "opentelemetry")]
                        trace::set_parent_from_envelope(&span, &envelope);

                        coordinator.handle_envelope(envelope).instrument(span).await;
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

    /// Handle received signaling envelope
    async fn handle_envelope(self: &Arc<Self>, envelope: SignalingEnvelope) {
        // Decode SignalingEnvelope
        match envelope.flow {
            Some(signaling_envelope::Flow::ActrRelay(relay)) => {
                let source = relay.source;
                let target = relay.target;
                let source_repr = source.to_string_repr();
                match relay.payload {
                    Some(actr_relay::Payload::SessionDescription(sd)) => match sd.r#type() {
                        SdpType::Offer => {
                            tracing::info!("📥 Received Offer from {:?}", source.serial_number);
                            if let Err(e) = self.handle_offer(&source, sd.sdp).await {
                                tracing::error!("❌ Failed to handle Offer: {}", e);
                            }
                        }
                        SdpType::Answer => {
                            tracing::info!("📥 Received Answer from {:?}", source.serial_number);
                            if let Err(e) = self.handle_answer(&source, sd.sdp).await {
                                tracing::error!("❌ Failed to handle Answer: {}", e);
                            }
                        }
                        SdpType::RenegotiationOffer => {
                            tracing::warn!("⚠️ Received RenegotiationOffer, not supported yet");
                        }
                        SdpType::IceRestartOffer => {
                            tracing::info!(
                                "♻️ Received ICE Restart Offer from {:?}",
                                source.serial_number
                            );
                            if let Err(e) = self.handle_ice_restart_offer(&source, sd.sdp).await {
                                tracing::error!("❌ Failed to handle ICE Restart Offer: {}", e);
                            }
                        }
                    },
                    Some(actr_relay::Payload::RoleAssignment(assign)) => {
                        tracing::info!(
                            "🎭 Received RoleAssignment from {:?}, is_offerer={} (source peer)",
                            source.serial_number,
                            assign.is_offerer,
                        );
                        let peer = if source == self.local_id {
                            target.clone()
                        } else {
                            source.clone()
                        };
                        self.handle_role_assignment(assign.clone(), peer).await;
                    }
                    Some(actr_relay::Payload::IceCandidate(ice)) => {
                        tracing::debug!(
                            "📥 Received ICE Candidate from {:?}",
                            source.serial_number
                        );
                        if let Err(e) = self.handle_ice_candidate(&source, ice.candidate).await {
                            tracing::error!("❌ Failed to handle ICE Candidate: {}", e);
                        }
                    }
                    Some(actr_relay::Payload::RoleNegotiation(_)) => {
                        tracing::trace!(
                            "📥 Received RoleNegotiation payload; ignored by WebRtcCoordinator"
                        );
                    }
                    None => {
                        tracing::warn!("⚠️ ActrRelay missing payload");
                    }
                }
            }
            Some(other_flow) => {
                tracing::warn!("⚠️ Ignoring non-ActrRelay flow: {:?}", other_flow);
            }
            None => {
                tracing::warn!("⚠️ SignalingEnvelope missing flow");
            }
        }
    }

    /// Close all peer connections and clear internal peer state.
    ///
    /// This is typically called during shutdown to ensure that all
    /// RTCPeerConnection instances are closed and associated state
    /// (pending ICE candidates, WebRtcConnection state) is dropped.
    pub async fn close_all_peers(&self) -> RuntimeResult<()> {
        tracing::info!("🔻 Closing all WebRTC peer connections");

        // Take snapshot of peers and clear map
        let peers_snapshot: Vec<Arc<RTCPeerConnection>> = {
            let mut peers = self.peers.write().await;
            let conns: Vec<Arc<RTCPeerConnection>> =
                peers.values().map(|p| p.peer_connection.clone()).collect();
            peers.clear();
            conns
        };

        // Clear pending ICE candidates
        {
            let mut pending = self.pending_candidates.write().await;
            pending.clear();
        }

        // Clear root tracing contexts (if enabled)
        #[cfg(feature = "opentelemetry")]
        {
            let mut ctx_map = self.root_context_map.write().await;
            ctx_map.clear();
        }

        // Close each RTCPeerConnection
        for pc in peers_snapshot {
            tracing::info!("🔻 Closing PeerConnection");

            if let Err(e) = pc.close().await {
                tracing::warn!("⚠️ Failed to close PeerConnection: {}", e);
            } else {
                tracing::info!("✅ PeerConnection closed");
            }
        }

        Ok(())
    }

    /// Send ActrRelay message (internal helper method)
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(target = %target.to_string_repr())
    )]
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
            traceparent: None,
            tracestate: None,
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
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            target_id = %target.to_string_repr()
        )
    )]
    pub async fn initiate_connection(
        self: &Arc<Self>,
        target: &ActrId,
    ) -> RuntimeResult<oneshot::Receiver<()>> {
        tracing::info!(
            "🚀 Initiating P2P connection to {}",
            target.to_string_repr()
        );

        // 0. 若已有协商结果，直接用；否则执行协商
        if let Some(is_offerer) = self.negotiated_role.lock().await.remove(target) {
            if is_offerer {
                return self.start_offer_connection(target, true).await;
            } else {
                let (tx, rx) = oneshot::channel();
                self.pending_ready.lock().await.insert(target.clone(), tx);
                return Ok(rx);
            }
        }

        // 0. Role negotiation: only proceed as offerer
        let role_result =
            tokio::time::timeout(Duration::from_secs(5), self.negotiate_role(target)).await;

        let role_result = match role_result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                self.pending_role.lock().await.remove(target);
                return Err(e);
            }
            Err(_) => {
                self.pending_role.lock().await.remove(target);
                return Err(RuntimeError::DeadlineExceeded {
                    message: "Role negotiation timeout1".to_string(),
                    timeout_ms: 5000,
                });
            }
        };
        debug!("role_result: {:?}", role_result);
        if !role_result {
            tracing::info!(
                "🎭 Role negotiation decided we are answerer for {}, waiting for offer",
                target.serial_number
            );
            let (tx, rx) = oneshot::channel();
            self.pending_ready.lock().await.insert(target.clone(), tx);
            tracing::debug!("start_offer_connection: answerer waiting for offer");
            return Ok(rx);
        }

        self.start_offer_connection(target, true).await
    }

    /// Create and send an offer (offerer path). If `skip_negotiation` is true, assumes角色已确定。
    /// This method includes retry logic for initial connection failures.
    async fn start_offer_connection(
        self: &Arc<Self>,
        target: &ActrId,
        skip_negotiation: bool,
    ) -> RuntimeResult<oneshot::Receiver<()>> {
        if !skip_negotiation {
            let role_result =
                tokio::time::timeout(Duration::from_secs(15), self.negotiate_role(target)).await;

            let role_result = match role_result {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => {
                    self.pending_role.lock().await.remove(target);
                    return Err(e);
                }
                Err(_) => {
                    self.pending_role.lock().await.remove(target);
                    return Err(RuntimeError::DeadlineExceeded {
                        message: "Role negotiation timeout".to_string(),
                        timeout_ms: 5000,
                    });
                }
            };

            if !role_result {
                tracing::info!(
                    "🎭 Role negotiation decided we are answerer for {}, waiting for offer",
                    target.serial_number
                );
                let (tx, rx) = oneshot::channel();
                self.pending_ready.lock().await.insert(target.clone(), tx);
                return Ok(rx);
            }
        }

        // Retry loop for initial connection
        let backoff = ExponentialBackoff::new(
            Duration::from_millis(INITIAL_CONNECTION_INITIAL_BACKOFF_MS),
            Duration::from_millis(INITIAL_CONNECTION_MAX_BACKOFF_MS),
            Some(INITIAL_CONNECTION_MAX_RETRIES),
        );

        let mut last_error: Option<RuntimeError> = None;
        let mut attempt = 0u32;

        for delay in backoff {
            attempt += 1;
            tracing::info!(
                "🔄 Connection attempt {} to serial={}",
                attempt,
                target.serial_number
            );

            match self.do_single_offer_connection(target).await {
                Ok((ready_rx, webrtc_conn)) => {
                    // Wait for connection to be ready with timeout
                    match tokio::time::timeout(INITIAL_CONNECTION_TIMEOUT, ready_rx).await {
                        Ok(Ok(())) => {
                            tracing::info!(
                                "✅ Connection established to serial={} on attempt {}",
                                target.serial_number,
                                attempt
                            );
                            // Return a new channel that's already signaled
                            let (tx, rx) = oneshot::channel();
                            let _ = tx.send(());
                            return Ok(rx);
                        }
                        Ok(Err(_)) => {
                            tracing::warn!(
                                "⚠️ Connection attempt {} failed (channel closed) for serial={}",
                                attempt,
                                target.serial_number
                            );
                            last_error = Some(RuntimeError::Other(anyhow::anyhow!(
                                "Connection ready channel closed"
                            )));
                        }
                        Err(_) => {
                            tracing::warn!(
                                "⚠️ Connection attempt {} timed out for serial={}",
                                attempt,
                                target.serial_number
                            );
                            last_error = Some(RuntimeError::DeadlineExceeded {
                                message: format!(
                                    "Initial connection timeout on attempt {}",
                                    attempt
                                ),
                                timeout_ms: INITIAL_CONNECTION_TIMEOUT.as_millis() as u64,
                            });
                        }
                    }

                    // Cleanup failed connection attempt
                    self.cleanup_failed_connection(target, webrtc_conn).await;
                }
                Err(e) => {
                    tracing::warn!(
                        "⚠️ Connection attempt {} failed for serial={}: {}",
                        attempt,
                        target.serial_number,
                        e
                    );
                    last_error = Some(e);
                }
            }

            // Wait before next retry
            tracing::info!(
                "⏳ Waiting {:?} before next connection attempt to serial={}",
                delay,
                target.serial_number
            );
            tokio::time::sleep(delay).await;
        }

        // All retries exhausted
        tracing::error!(
            "❌ All {} connection attempts failed for serial={}",
            INITIAL_CONNECTION_MAX_RETRIES,
            target.serial_number
        );

        Err(last_error.unwrap_or_else(|| {
            RuntimeError::Other(anyhow::anyhow!("Connection failed after all retries"))
        }))
    }

    /// Cleanup a failed connection attempt
    async fn cleanup_failed_connection(&self, target: &ActrId, webrtc_conn: WebRtcConnection) {
        // Remove from peers map
        {
            let mut peers = self.peers.write().await;
            if let Some(state) = peers.remove(target) {
                if let Err(e) = state.peer_connection.close().await {
                    tracing::warn!(
                        "⚠️ Failed to close peer_connection during cleanup for {}: {}",
                        target.serial_number,
                        e
                    );
                }
            }
        }

        // Close WebRtcConnection
        if let Err(e) = webrtc_conn.close().await {
            tracing::warn!(
                "⚠️ Failed to close WebRtcConnection during cleanup for {}: {}",
                target.serial_number,
                e
            );
        }

        // Clear pending candidates
        {
            let mut pending = self.pending_candidates.write().await;
            pending.remove(target);
        }

        tracing::debug!(
            "🧹 Cleaned up failed connection attempt for serial={}",
            target.serial_number
        );
    }

    /// Perform a single offer connection attempt (without retry logic)
    async fn do_single_offer_connection(
        self: &Arc<Self>,
        target: &ActrId,
    ) -> RuntimeResult<(oneshot::Receiver<()>, WebRtcConnection)> {
        let peer_connection = self.negotiator.create_peer_connection().await?;
        let peer_connection_arc = Arc::new(peer_connection);

        // 2. Create WebRtcConnection (shares Arc<RTCPeerConnection>) and
        //    install state-change handler with ICE-restart wiring.
        let webrtc_conn = WebRtcConnection::new(Arc::clone(&peer_connection_arc));
        self.install_restart_handler(
            webrtc_conn.clone(),
            Arc::clone(&peer_connection_arc),
            target.clone(),
        );
        self.register_data_channel_cleanup(&webrtc_conn, target)
            .await;

        // 3. Pre-create negotiated DataChannel for Reliable to trigger ICE gathering
        let _reliable_lane = webrtc_conn
            .get_lane(actr_protocol::PayloadType::RpcReliable)
            .await?;
        tracing::debug!("Pre-created Reliable DataChannel for ICE gathering");

        // 3.5. Pre-create media tracks for sending (MUST be done before creating Offer)
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
                    sender_id.to_string_repr()
                );

                tokio::spawn(async move {
                    loop {
                        match track.read_rtp().await {
                            Ok((rtp_packet, _attributes)) => {
                                let payload_data = rtp_packet.payload.clone();
                                let timestamp = rtp_packet.header.timestamp;
                                let codec = "unknown".to_string();
                                let sample = actr_framework::MediaSample {
                                    data: payload_data,
                                    timestamp,
                                    codec,
                                    media_type: actr_framework::MediaType::Video,
                                };
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
        #[cfg(feature = "opentelemetry")]
        let root_context_map = self.root_context_map.clone();
        peer_connection_arc.on_ice_candidate(Box::new(
            move |candidate: Option<RTCIceCandidate>| {
                let coordinator = coordinator.clone();
                let target_id = target_id.clone();
                #[cfg(feature = "opentelemetry")]
                let root_context_map = root_context_map.clone();
                Box::pin(async move {
                    if let Some(cand) = candidate {
                        if let Some(coord) = coordinator.upgrade() {
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
                            let span = tracing::info_span!(
                                "send_ice_candidate",
                                target_id = %target_id.to_string_repr()
                            );
                            // Get root context at callback execution time (not at setup time)
                            #[cfg(feature = "opentelemetry")]
                            if let Some(ctx) =
                                root_context_map.read().await.get(&target_id).cloned()
                            {
                                span.set_parent(ctx);
                            }

                            if let Err(e) = coord
                                .send_actr_relay(&target_id, payload)
                                .instrument(span)
                                .await
                            {
                                tracing::error!("❌ Failed to send ICE Candidate: {}", e);
                            } else {
                                tracing::debug!("✅ Sent ICE Candidate");
                            }
                        }
                    } else {
                        tracing::debug!("❌ ICE Candidate is None");
                    }
                })
            },
        ));

        // 6. Create Offer
        let offer_sdp = self.negotiator.create_offer(&peer_connection_arc).await?;

        // 7. Create ready notification channel
        let (ready_tx, ready_rx) = oneshot::channel();

        // 8. Store peer state BEFORE sending Offer (prevent race condition)
        {
            let mut peers = self.peers.write().await;
            tracing::info!(
                "🔧 [STORE] Inserting peer: id={}, current peers={}",
                target.to_string_repr(),
                peers.len()
            );
            peers.insert(
                target.clone(),
                PeerState {
                    peer_connection: peer_connection_arc.clone(),
                    webrtc_conn: webrtc_conn.clone(),
                    ready_tx: Some(ready_tx),
                    is_offerer: true,
                    ice_restart_inflight: false,
                    ice_restart_attempts: 0,
                },
            );
            tracing::info!("✅ [STORE] Peer inserted, new total={}", peers.len());
        }

        // 9. Send Offer via signaling server (AFTER storing peer state)
        let session_desc = actr_protocol::SessionDescription {
            r#type: SdpType::Offer as i32,
            sdp: offer_sdp,
        };
        let payload = actr_relay::Payload::SessionDescription(session_desc);
        self.send_actr_relay(target, payload).await?;

        tracing::info!("✅ Sent Offer to {}", target.to_string_repr());

        // 10. Start receive loop (receive and aggregate messages from this peer)
        self.start_peer_receive_loop(target.clone(), webrtc_conn.clone())
            .await;

        Ok((ready_rx, webrtc_conn))
    }

    /// Handle received Offer (passive side)
    ///
    /// Called when receiving a connection request from another peer.
    /// Supports both initial negotiation and renegotiation.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            remote_id = %from.to_string_repr()
        )
    )]
    async fn handle_offer(self: &Arc<Self>, from: &ActrId, offer_sdp: String) -> RuntimeResult<()> {
        // ========== PrepareForIncomingOffer: Clean up existing connection if any ==========
        let existing_peer = {
            let peers = self.peers.read().await;
            peers.contains_key(from)
        };

        if existing_peer {
            tracing::info!(
                "🔄 Existing connection found for serial={}, preparing for new Offer",
                from.serial_number
            );

            // 1. Cancel in-flight ICE restart task
            {
                let mut map = self.in_flight_restarts.lock().await;
                if let Some(handle) = map.remove(from) {
                    handle.abort();
                    tracing::debug!(
                        "🧹 Aborted in-flight ICE restart due to incoming Offer from serial={}",
                        from.serial_number
                    );
                }
            }

            // 2. Close old connection
            {
                let mut peers = self.peers.write().await;
                if let Some(old_state) = peers.remove(from) {
                    if let Err(e) = old_state.peer_connection.close().await {
                        tracing::warn!(
                            "⚠️ Failed to close old peer_connection for {}: {}",
                            from.serial_number,
                            e
                        );
                    }
                    if let Err(e) = old_state.webrtc_conn.close().await {
                        tracing::warn!(
                            "⚠️ Failed to close old WebRtcConnection for {}: {}",
                            from.serial_number,
                            e
                        );
                    }
                    tracing::info!(
                        "🧹 Cleaned up old connection for serial={}",
                        from.serial_number
                    );
                }
            }

            // 3. Clear pending ICE candidates
            {
                let mut pending = self.pending_candidates.write().await;
                if pending.remove(from).is_some() {
                    tracing::debug!(
                        "🧹 Cleared pending ICE candidates for serial={}",
                        from.serial_number
                    );
                }
            }

            // 4. Clear role negotiation caches
            {
                self.negotiated_role.lock().await.remove(from);
                self.pending_role.lock().await.remove(from);
                self.pending_ready.lock().await.remove(from);
                self.pending_ready_wait.lock().await.remove(from);
            }
        }
        // ========== PrepareForIncomingOffer END ==========

        tracing::info!("📥 Handling Offer from serial={}", from.serial_number);

        // 1. Create RTCPeerConnection
        let peer_connection = self.negotiator.create_peer_connection().await?;
        let peer_connection_arc = Arc::new(peer_connection);

        // 2. Create WebRtcConnection (shares Arc<RTCPeerConnection>) and
        //    install state-change handler for cleanup on terminal states.
        let webrtc_conn = WebRtcConnection::new(Arc::clone(&peer_connection_arc));
        webrtc_conn.install_state_change_handler();
        self.register_data_channel_cleanup(&webrtc_conn, from).await;

        // 3. Register on_data_channel handler to reuse negotiated channels created by the offerer
        let conn_for_data_channel = webrtc_conn.clone();
        let coord_weak = Arc::downgrade(self);
        let from_id = from.clone();
        peer_connection_arc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            let conn = conn_for_data_channel.clone();
            let coord_weak = coord_weak.clone();
            let target_id = from_id.clone();
            Box::pin(async move {
                let channel_id = dc.id();
                let label = dc.label();
                let dc_for_registration = Arc::clone(&dc);

                let payload_type = PayloadType::try_from(i32::from(channel_id)).ok();

                match payload_type {
                    Some(pt) => {
                        if let Err(e) = conn
                            .register_received_data_channel(dc_for_registration, pt)
                            .await
                        {
                            tracing::warn!(
                                "❌ Failed to register received DataChannel label={} id={}: {}",
                                label,
                                channel_id,
                                e
                            );
                        } else {
                            tracing::debug!(
                                "📨 Registered DataChannel from offerer label={} id={}",
                                label,
                                channel_id
                            );
                            // If this side was waiting as answerer, notify readiness once DC is registered
                            if let Some(coord) = coord_weak.upgrade() {
                                if let Some(tx) =
                                    coord.pending_ready.lock().await.remove(&target_id)
                                {
                                    let _ = tx.send(());
                                }
                            }
                        }
                    }
                    None => {
                        tracing::warn!(
                            "❓ Ignoring DataChannel with unmapped id={} label={}",
                            channel_id,
                            label
                        );
                    }
                }
            })
        }));

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
                    sender_id.to_string_repr()
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
        #[cfg(feature = "opentelemetry")]
        let root_context_map = self.root_context_map.clone();
        peer_connection_arc.on_ice_candidate(Box::new(
            move |candidate: Option<RTCIceCandidate>| {
                let coordinator = coordinator.clone();
                let target_id = target_id.clone();
                #[cfg(feature = "opentelemetry")]
                let root_context_map = root_context_map.clone();
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
                            let span = tracing::info_span!(
                                "send_ice_candidate",
                                target_id = %target_id.to_string_repr()
                            );
                            // Get root context at callback execution time (not at setup time)
                            #[cfg(feature = "opentelemetry")]
                            if let Some(ctx) =
                                root_context_map.read().await.get(&target_id).cloned()
                            {
                                span.set_parent(ctx);
                            }
                            if let Err(e) = coord
                                .send_actr_relay(&target_id, payload)
                                .instrument(span)
                                .await
                            {
                                tracing::error!("❌ Failed to send ICE Candidate: {}", e);
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
                    is_offerer: false,
                    ice_restart_inflight: false,
                    ice_restart_attempts: 0,
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

        tracing::info!("✅ Sent Answer to {}", from.to_string_repr());

        // 8. Flush any buffered ICE candidates (remote description is now set)
        self.flush_pending_candidates(from, &peer_connection_arc)
            .await?;

        // 9. Start receive loop
        self.start_peer_receive_loop(from.clone(), webrtc_conn)
            .await;

        // 如果有等待中的 ready（answerer 发起路径），此时可以标记就绪
        if let Some(tx) = self.pending_ready.lock().await.remove(from) {
            tracing::debug!("🔄 Sending ready notification to answerer: {:?}", from);
            let _ = tx.send(());
        }

        Ok(())
    }

    /// Handle received Answer (initiator side)
    ///
    /// Supports both initial negotiation and renegotiation answers.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            remote.id = %from.to_string_repr(),
            answer_len = answer_sdp.len()
        )
    )]
    async fn handle_answer(
        self: &Arc<Self>,
        from: &ActrId,
        answer_sdp: String,
    ) -> RuntimeResult<()> {
        // Get corresponding PeerConnection and ready_tx
        let (peer_connection, ready_tx, is_renegotiation) = {
            let mut peers = self.peers.write().await;
            tracing::info!(
                "🔍 [LOOKUP] Searching for: id={}, total peers={}",
                from.to_string_repr(),
                peers.len()
            );
            for (k, _) in peers.iter() {
                tracing::info!("   📌 [LOOKUP] Stored: id={}", k.to_string_repr());
            }
            let state = peers.get_mut(from).ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!("Peer not found: {}", from.to_string_repr()))
            })?;

            let pc = state.peer_connection.clone();
            let tx = state.ready_tx.take();
            let is_reneg = tx.is_none(); // If ready_tx already taken, this is renegotiation
            (pc, tx, is_reneg)
        };

        if is_renegotiation {
            tracing::info!(
                "🔄 Handling renegotiation Answer from {}",
                from.to_string_repr()
            );
        } else {
            tracing::info!("📥 Handling initial Answer from {}", from.to_string_repr());
        }

        // Handle Answer (set remote SDP)
        self.negotiator
            .handle_answer(&peer_connection, answer_sdp)
            .await?;

        // Flush any buffered ICE candidates (remote description is now set)
        self.flush_pending_candidates(from, &peer_connection)
            .await?;

        tracing::info!(
            "✅ WebRTC connection negotiation completed: {}",
            from.to_string_repr()
        );

        // Wait for PeerConnection to actually connect (max 5 seconds)
        let pc_clone = peer_connection.clone();
        let peers = Arc::clone(&self.peers);
        let from_id = from.clone();
        tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            loop {
                let state = pc_clone.connection_state();
                if state == webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Connected {
                    tracing::info!("✅ PeerConnection fully connected");
                    // Mark ICE restart attempt complete
                    let mut peers_guard = peers.write().await;
                    if let Some(s) = peers_guard.get_mut(&from_id) {
                        s.ice_restart_inflight = false;
                        s.ice_restart_attempts = 0;
                    }
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
    #[tracing::instrument(
        level = "trace",
        skip_all,
        fields(
            remote.id = %from.to_string_repr(),
            candidate_len = candidate.len()
        )
    )]
    async fn handle_ice_candidate(
        self: &Arc<Self>,
        from: &ActrId,
        candidate: String,
    ) -> RuntimeResult<()> {
        tracing::trace!("📥 Received ICE Candidate from {}", from.to_string_repr());

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
                    tracing::trace!("✅ Added ICE Candidate from {}", from.to_string_repr());
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
                            if let Err(e) =
                                message_tx_clone.send((peer_id_bytes, data, payload_type))
                            {
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
    pub(crate) async fn send_message(
        self: &Arc<Self>,
        target: &ActrId,
        data: &[u8],
    ) -> RuntimeResult<()> {
        tracing::debug!("📤 Sending message to {:?}: {} bytes", target, data.len());

        // Check if connection exists
        let has_connection = {
            let peers = self.peers.read().await;
            peers.contains_key(target)
        };

        let root_span =
            tracing::info_span!("initiate_connection", target_id = %target.to_string_repr());
        #[cfg(feature = "opentelemetry")]
        {
            let root_ctx = root_span.context();
            self.root_context_map
                .write()
                .await
                .insert(target.clone(), root_ctx);
        }

        // If connection doesn't exist, initiate connection
        if !has_connection {
            tracing::info!(
                "🔗 First send to {:?}, initiating role negotiation + WebRTC connection",
                target.serial_number
            );

            // 先查是否已有协商结果（可能来自对端主动 RoleAssignment）
            let role_hint = self.negotiated_role.lock().await.remove(target);
            let ready_rx = if let Some(is_offerer) = role_hint {
                if is_offerer {
                    self.start_offer_connection(target, true).await?
                } else {
                    let (tx, rx) = oneshot::channel();
                    self.pending_ready.lock().await.insert(target.clone(), tx);
                    rx
                }
            } else {
                // 正常流程：发起协商并按结果建链
                let ready_rx = self
                    .initiate_connection(target)
                    .instrument(root_span)
                    .await?;
                ready_rx
            };
            tracing::debug!(?ready_rx, "ready_rx");
            // Wait for connection to be ready (30s timeout)
            match tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx).await {
                Ok(Ok(())) => {
                    tracing::info!("✅ WebRTC connection ready: {}", target.to_string_repr());
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
            target_id.to_string_repr()
        );

        // 2. Check if connection already exists
        {
            let peers = self.peers.read().await;
            if let Some(state) = peers.get(target_id) {
                tracing::debug!(
                    "♻️ [Factory] Reusing existing WebRTC connection: {:?}",
                    target_id.to_string_repr()
                );
                return Ok(state.webrtc_conn.clone());
            }
        }

        // 3. Initiate new connection
        tracing::info!(
            "🔨 [Factory] Initiating new WebRTC connection: {:?}",
            target_id.to_string_repr()
        );
        let root_span =
            tracing::info_span!("create_connection", target_id = %target_id.to_string_repr());
        #[cfg(feature = "opentelemetry")]
        {
            let root_ctx = root_span.context();
            self.root_context_map
                .write()
                .await
                .insert(target_id.clone(), root_ctx);
        }

        let ready_rx = self
            .initiate_connection(target_id)
            .instrument(root_span)
            .await?;

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
            target_id.to_string_repr()
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
                "No connection to target: {}",
                target.to_string_repr()
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
            target.to_string_repr()
        );

        // 1. Get existing peer state and extract needed parts
        let (webrtc_conn, peer_connection) = {
            let peers = self.peers.read().await;
            let state = peers.get(target).ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!(
                    "No connection to target: {}",
                    target.to_string_repr()
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
        let root_span = tracing::info_span!("add_track", target_id = %target.to_string_repr());
        #[cfg(feature = "opentelemetry")]
        {
            let root_ctx = root_span.context();
            self.root_context_map
                .write()
                .await
                .insert(target.clone(), root_ctx);
        }
        self.renegotiate_connection(target, &peer_connection)
            .instrument(root_span)
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
            "🔄 Starting SDP renegotiation with {}",
            target.to_string_repr()
        );

        // 1. Create new Offer (includes all tracks: old + new)
        let offer = peer_connection.create_offer(None).await.map_err(|e| {
            RuntimeError::Other(anyhow::anyhow!("Failed to create renegotiation offer: {e}"))
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

        tracing::info!("✅ Sent renegotiation Offer to {}", target.to_string_repr());

        // 4. Answer will be handled by existing handle_answer() method
        // Note: We don't wait for Answer here to avoid blocking.
        // The renegotiation completes asynchronously when Answer arrives.

        Ok(())
    }

    /// Initiate ICE restart on an existing connection (offerer side).
    /// Uses in_flight_restarts to de-duplicate concurrent restart requests.
    /// If ICE restart fails after all retries, attempts to establish a new connection.
    pub async fn restart_ice(
        self: &Arc<Self>,
        target: &actr_protocol::ActrId,
    ) -> RuntimeResult<()> {
        // 1. Check if restart is already in-flight (de-duplication)
        {
            let map = self.in_flight_restarts.lock().await;
            if let Some(handle) = map.get(target) {
                if !handle.is_finished() {
                    tracing::debug!(
                        "🚫 ICE restart already in-flight for serial={}, skipping",
                        target.serial_number
                    );
                    return Ok(());
                }
            }
        }

        // 2. Check if we are the offerer and peer exists
        let peer_connection = {
            let peers = self.peers.read().await;
            if let Some(state) = peers.get(target) {
                if !state.is_offerer {
                    tracing::warn!(
                        "🚫 Skip ICE restart to serial={}: we are not the offerer",
                        target.serial_number
                    );
                    return Ok(());
                }
                state.peer_connection.clone()
            } else {
                tracing::warn!(
                    "🚫 Skip ICE restart to serial={}: peer not found",
                    target.serial_number
                );
                return Ok(());
            }
        };

        tracing::info!(
            "♻️ Initiating ICE restart to serial={}",
            target.serial_number
        );

        // 3. Spawn restart task and track it
        let target_clone = target.clone();
        let peers = Arc::clone(&self.peers);
        let negotiator = self.negotiator.clone();
        let in_flight_cleanup = Arc::clone(&self.in_flight_restarts);
        let local_id = self.local_id.clone();
        let credential = self.credential.clone();
        let signaling_client = Arc::clone(&self.signaling_client);
        let coordinator_weak = Arc::downgrade(self);

        let handle = tokio::spawn(async move {
            let restart_result = Self::do_ice_restart_inner(
                &target_clone,
                &peers,
                peer_connection,
                &negotiator,
                &local_id,
                &credential,
                &signaling_client,
            )
            .await;

            match restart_result {
                Ok(true) => {
                    tracing::info!(
                        "✅ ICE restart succeeded for serial={}",
                        target_clone.serial_number
                    );
                }
                Ok(false) => {
                    // ICE restart failed after all retries, try to establish new connection
                    tracing::warn!(
                        "⚠️ ICE restart exhausted for serial={}, attempting fresh connection",
                        target_clone.serial_number
                    );

                    if let Some(coord) = coordinator_weak.upgrade() {
                        // Attempt to establish a new connection
                        match coord.start_offer_connection(&target_clone, true).await {
                            Ok(ready_rx) => {
                                // Wait for the new connection to be ready
                                match tokio::time::timeout(INITIAL_CONNECTION_TIMEOUT, ready_rx)
                                    .await
                                {
                                    Ok(Ok(())) => {
                                        tracing::info!(
                                            "✅ Fresh connection established after ICE restart failure for serial={}",
                                            target_clone.serial_number
                                        );
                                    }
                                    Ok(Err(_)) => {
                                        tracing::error!(
                                            "❌ Fresh connection failed (channel closed) for serial={}",
                                            target_clone.serial_number
                                        );
                                    }
                                    Err(_) => {
                                        tracing::error!(
                                            "❌ Fresh connection timed out for serial={}",
                                            target_clone.serial_number
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "❌ Failed to initiate fresh connection after ICE restart failure for serial={}: {}",
                                    target_clone.serial_number,
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "❌ ICE restart failed for serial={}: {}",
                        target_clone.serial_number,
                        e
                    );
                }
            }

            // Cleanup registration
            in_flight_cleanup.lock().await.remove(&target_clone);
        });

        self.in_flight_restarts
            .lock()
            .await
            .insert(target.clone(), handle);

        Ok(())
    }

    /// Internal ICE restart implementation with retries
    /// Returns Ok(true) if restart succeeded, Ok(false) if all retries exhausted
    async fn do_ice_restart_inner(
        target: &ActrId,
        peers: &Arc<RwLock<HashMap<ActrId, PeerState>>>,
        peer_connection: Arc<RTCPeerConnection>,
        negotiator: &WebRtcNegotiator,
        local_id: &ActrId,
        credential: &AIdCredential,
        signaling_client: &Arc<dyn SignalingClient>,
    ) -> RuntimeResult<bool> {
        let backoff = ExponentialBackoff::new(
            Duration::from_millis(ICE_RESTART_INITIAL_BACKOFF_MS),
            Duration::from_millis(ICE_RESTART_MAX_BACKOFF_MS),
            Some(ICE_RESTART_MAX_RETRIES),
        );

        let mut restart_ok = false;

        for delay in backoff {
            let (offer_sdp, attempt) = {
                let mut peers_guard = peers.write().await;
                let state = match peers_guard.get_mut(target) {
                    Some(s) => s,
                    None => {
                        tracing::warn!(
                            "🚫 Peer state not found during ICE restart for serial={}",
                            target.serial_number
                        );
                        return Ok(false);
                    }
                };

                if !state.is_offerer {
                    tracing::warn!(
                        "🚫 Skip ICE restart to serial={}: we are not the offerer",
                        target.serial_number
                    );
                    state.ice_restart_inflight = false;
                    state.ice_restart_attempts = 0;
                    return Ok(false);
                }

                state.ice_restart_attempts += 1;
                state.ice_restart_inflight = true;
                let attempt = state.ice_restart_attempts;

                let offer_sdp = negotiator
                    .create_ice_restart_offer(&peer_connection)
                    .await?;

                (offer_sdp, attempt)
            };

            // Send ICE restart offer
            let relay = ActrRelay {
                source: local_id.clone(),
                credential: credential.clone(),
                target: target.clone(),
                payload: Some(actr_relay::Payload::SessionDescription(
                    actr_protocol::SessionDescription {
                        r#type: SdpType::IceRestartOffer as i32,
                        sdp: offer_sdp,
                    },
                )),
            };

            let envelope = SignalingEnvelope {
                envelope_version: 1,
                envelope_id: uuid::Uuid::new_v4().to_string(),
                reply_for: None,
                timestamp: prost_types::Timestamp {
                    seconds: chrono::Utc::now().timestamp(),
                    nanos: 0,
                },
                flow: Some(signaling_envelope::Flow::ActrRelay(relay)),
                traceparent: None,
                tracestate: None,
            };

            if let Err(e) = signaling_client.send_envelope(envelope).await {
                tracing::error!(
                    "❌ Failed to send ICE restart offer to serial={}: {}",
                    target.serial_number,
                    e
                );
                // Mark inflight as false and continue to next retry
                let mut peers_guard = peers.write().await;
                if let Some(state) = peers_guard.get_mut(target) {
                    state.ice_restart_inflight = false;
                }
                tokio::time::sleep(delay).await;
                continue;
            }

            tracing::info!(
                "♻️ ICE restart attempt {} sent to serial={}",
                attempt,
                target.serial_number
            );

            // Wait for restart completion
            let success =
                Self::wait_for_restart_completion_static(peers, target, ICE_RESTART_TIMEOUT).await;

            if success {
                restart_ok = true;
                break;
            }

            tracing::warn!(
                "⚠️ ICE restart attempt {} timed out for serial={}",
                attempt,
                target.serial_number
            );

            // Mark current attempt ended
            {
                let mut peers_guard = peers.write().await;
                if let Some(state) = peers_guard.get_mut(target) {
                    state.ice_restart_inflight = false;
                }
            }

            // Exponential backoff before retrying
            tracing::info!(
                "⏳ Waiting {:?} before next ICE restart attempt to serial={}",
                delay,
                target.serial_number
            );
            tokio::time::sleep(delay).await;
        }

        if !restart_ok {
            tracing::warn!(
                "⚠️ Backoff iterator exhausted for serial={}, stopping retries and dropping peer",
                target.serial_number
            );
            Self::drop_peer_connection_static(peers, target).await;
            return Ok(false);
        }

        Ok(true)
    }

    /// Static version of wait_for_restart_completion for use in spawned task
    /// Uses read lock for checking status to avoid blocking other peers
    async fn wait_for_restart_completion_static(
        peers: &Arc<RwLock<HashMap<ActrId, PeerState>>>,
        target: &ActrId,
        timeout: Duration,
    ) -> bool {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let timeout_sleep = tokio::time::sleep(timeout);
        tokio::pin!(timeout_sleep);

        loop {
            tokio::select! {
                _ = &mut timeout_sleep => {
                    return false;
                }
                _ = interval.tick() => {
                    // Use read lock to check status (allows concurrent access)
                    let is_done = {
                        let peers_guard = peers.read().await;
                        match peers_guard.get(target) {
                            Some(state) => !state.ice_restart_inflight,
                            None => return false,
                        }
                    };

                    if is_done {
                        // Only acquire write lock when actually need to reset counter
                        let mut peers_guard = peers.write().await;
                        if let Some(state) = peers_guard.get_mut(target) {
                            state.ice_restart_attempts = 0;
                        }
                        return true;
                    }
                }
            }
        }
    }

    /// Static version of drop_peer_connection for use in spawned task
    async fn drop_peer_connection_static(
        peers: &Arc<RwLock<HashMap<ActrId, PeerState>>>,
        target: &ActrId,
    ) {
        let mut peers_guard = peers.write().await;
        if let Some(state) = peers_guard.remove(target) {
            if let Err(e) = state.peer_connection.close().await {
                tracing::warn!(
                    "⚠️ Failed to close peer_connection for {}: {}",
                    target.serial_number,
                    e
                );
            }
            if let Err(e) = state.webrtc_conn.close().await {
                tracing::warn!(
                    "⚠️ Failed to close WebRtcConnection for {}: {}",
                    target.serial_number,
                    e
                );
            }
            tracing::info!("🧹 Dropped peer connection for {}", target.serial_number);
        } else {
            tracing::warn!(
                "⚠️ drop_peer_connection: peer not found {}",
                target.serial_number
            );
        }
    }

    /// Handle renegotiation Offer (existing connection)
    ///
    /// Called when receiving an Offer on an already-established connection.
    /// This happens when the remote peer adds/removes tracks dynamically.
    #[allow(dead_code)]
    async fn handle_renegotiation_offer(
        &self,
        from: &ActrId,
        offer_sdp: String,
    ) -> RuntimeResult<()> {
        tracing::info!(
            "🔄 Processing renegotiation Offer from {}",
            from.to_string_repr()
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
                RuntimeError::Other(anyhow::anyhow!("Failed to parse renegotiation offer: {e}"))
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

        tracing::info!("✅ Sent renegotiation Answer to {}", from.to_string_repr());

        // Note: on_track callback will automatically trigger for new remote tracks
        // No need to manually handle track additions here

        Ok(())
    }

    /// Handle ICE restart Offer on an existing connection.
    /// Only the answerer should accept restart; offerer-side restarts are initiated locally.
    async fn handle_ice_restart_offer(
        self: &Arc<Self>,
        from: &ActrId,
        offer_sdp: String,
    ) -> RuntimeResult<()> {
        // Locate peer state and ensure we are not the offerer
        let (peer_connection, is_offerer) = {
            let peers = self.peers.read().await;
            let state = peers.get(from).ok_or_else(|| {
                RuntimeError::Other(anyhow::anyhow!(
                    "ICE restart offer received for unknown peer"
                ))
            })?;
            (state.peer_connection.clone(), state.is_offerer)
        };

        if is_offerer {
            tracing::warn!(
                "🚫 Ignoring ICE restart offer from {:?}: we are current offerer",
                from.serial_number
            );
            return Ok(());
        }

        // Apply remote restart offer and generate answer
        let answer_sdp = self
            .negotiator
            .create_answer(&peer_connection, offer_sdp)
            .await?;

        // Send restart answer back
        let session_desc = actr_protocol::SessionDescription {
            r#type: SdpType::Answer as i32,
            sdp: answer_sdp,
        };
        let payload = actr_relay::Payload::SessionDescription(session_desc);
        self.send_actr_relay(from, payload).await?;

        // Flush any buffered ICE candidates collected before remote description was set
        self.flush_pending_candidates(from, &peer_connection)
            .await?;

        tracing::info!(
            "✅ Completed ICE restart answer to serial={}",
            from.serial_number
        );

        Ok(())
    }

    async fn register_data_channel_cleanup(
        self: &Arc<Self>,
        webrtc_conn: &WebRtcConnection,
        target: &ActrId,
    ) {
        let coord = Arc::downgrade(self);
        let target_id = target.clone();
        webrtc_conn
            .add_data_channel_close_handler(move |payload_type| {
                let coord = coord.clone();
                let target_id = target_id.clone();
                async move {
                    tracing::warn!(
                        "⚠️ DataChannel closed for peer {}, payload_type={:?}; tearing down connection",
                        target_id.serial_number,
                        payload_type
                    );
                    if let Some(coord) = coord.upgrade() {
                        if let Err(e) = coord.drop_peer_connection(&target_id).await {
                            tracing::warn!(
                                "⚠️ drop_peer_connection failed for {}: {}",
                                target_id.serial_number,
                                e
                            );
                        }
                    }
                }
            })
            .await;
    }

    /// Remove peer connection and clear associated cached state.
    async fn drop_peer_connection(&self, target: &ActrId) -> RuntimeResult<()> {
        // Cancel in-flight ICE restart task first
        {
            let mut map = self.in_flight_restarts.lock().await;
            if let Some(handle) = map.remove(target) {
                handle.abort();
                tracing::debug!(
                    "🧹 Aborted in-flight ICE restart for serial={}",
                    target.serial_number
                );
            }
        }

        // Then remove and close peer connection
        let mut peers = self.peers.write().await;
        if let Some(state) = peers.remove(target) {
            if let Err(e) = state.peer_connection.close().await {
                tracing::warn!(
                    "⚠️ Failed to close peer_connection for {}: {}",
                    target.serial_number,
                    e
                );
            }
            if let Err(e) = state.webrtc_conn.close().await {
                tracing::warn!(
                    "⚠️ Failed to close WebRtcConnection for {}: {}",
                    target.serial_number,
                    e
                );
            }
            tracing::info!("🧹 Dropped peer connection for {}", target.serial_number);
        } else {
            tracing::warn!(
                "⚠️ drop_peer_connection: peer not found {}",
                target.serial_number
            );
        }

        // Clear pending candidates and negotiation caches so future connections start cleanly.
        self.pending_candidates.write().await.remove(target);
        self.negotiated_role.lock().await.remove(target);
        self.pending_role.lock().await.remove(target);
        self.pending_ready.lock().await.remove(target);
        self.pending_ready_wait.lock().await.remove(target);

        Ok(())
    }

    /// Handle role assignment result
    async fn handle_role_assignment(self: &Arc<Self>, assign: RoleAssignment, peer: ActrId) {
        tracing::debug!(?assign, ?peer, "handle_role_assignment");
        // 先尝试唤醒等待的协商
        if let Some(sender) = self.pending_role.lock().await.remove(&peer) {
            if sender.send(assign.is_offerer).is_ok() {
                // 记录结果，避免后续重复协商
                self.negotiated_role
                    .lock()
                    .await
                    .insert(peer.clone(), assign.is_offerer);
                return;
            }
        }

        tracing::debug!(
            ?assign,
            ?peer,
            "handle_role_assignment: no pending negotiation"
        );
        // 缓存角色结果，避免后续 send_message/initiates 重新发起协商
        self.negotiated_role
            .lock()
            .await
            .insert(peer.clone(), assign.is_offerer);
        tracing::debug!(?assign, ?peer, "handle_role_assignment: cached role result");
        // 如果目前还没有连接，根据角色立即行动，避免依赖 send_message 才触发
        let has_connection = self.peers.read().await.contains_key(&peer);
        if has_connection {
            return;
        }
        if assign.is_offerer {
            tracing::info!(
                "🎭 Acting as offerer to {} per assignment (no pending negotiation)",
                peer.serial_number
            );
            // Spawn the offer connection in background to avoid blocking signaling loop
            let this = Arc::clone(self);
            let peer_clone = peer.clone();
            tokio::spawn(async move {
                match this.start_offer_connection(&peer_clone, true).await {
                    Ok(ready_rx) => {
                        this.pending_ready_wait
                            .lock()
                            .await
                            .insert(peer_clone.clone(), ready_rx);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "⚠️ Failed to start proactive offer connection to {}: {}",
                            peer_clone.serial_number,
                            e
                        );
                    }
                }
            });
        } else {
            tracing::debug!(
                "🎭 Assignment marks us as answerer for {}, waiting for offer (no pending negotiation)",
                peer.serial_number
            );
            let (tx, _rx) = oneshot::channel();
            self.pending_ready.lock().await.insert(peer.clone(), tx);

            // 防止长时间等不到 offer：超时后主动重新协商/建链
            let weak = Arc::downgrade(self);
            let peer_clone = peer.clone();
            tokio::spawn(async move {
                tokio::time::sleep(ROLE_WAIT_TIMEOUT).await;
                if let Some(coord) = weak.upgrade() {
                    // 如果已经有连接或 ready 被消费则退出
                    if coord.peers.read().await.contains_key(&peer_clone) {
                        return;
                    }
                    let pending = coord.pending_ready.lock().await.remove(&peer_clone);
                    if pending.is_none() {
                        return;
                    }
                    tracing::warn!(
                        "⏳ Waiting for offer from {} timed out, force acting as offerer",
                        peer_clone.serial_number
                    );
                    // 缓存强制角色，避免再次协商
                    coord
                        .negotiated_role
                        .lock()
                        .await
                        .insert(peer_clone.clone(), true);
                    match coord.start_offer_connection(&peer_clone, true).await {
                        Ok(ready_rx) => {
                            coord
                                .pending_ready_wait
                                .lock()
                                .await
                                .insert(peer_clone.clone(), ready_rx);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "⚠️ Failed to start offer connection after timeout to {}: {}",
                                peer_clone.serial_number,
                                e
                            );
                        }
                    }
                }
            });
        }
    }

    /// Initiate role negotiation and await assignment
    async fn negotiate_role(&self, target: &ActrId) -> RuntimeResult<bool> {
        let (tx, rx) = oneshot::channel();
        // 按目标 ActorId 记录等待的角色分配
        self.pending_role.lock().await.insert(target.clone(), tx);

        let payload = actr_relay::Payload::RoleNegotiation(RoleNegotiation {
            from: self.local_id.clone(),
            to: target.clone(),
            tenant_id: self.local_id.realm.realm_id.to_string(),
        });
        self.send_actr_relay(target, payload).await?;

        rx.await.map_err(|_| {
            RuntimeError::Other(anyhow::anyhow!(
                "Role negotiation channel closed before assignment"
            ))
        })
    }

    /// Install a state change handler to auto-trigger ICE restart on disconnection (offerer only).
    fn install_restart_handler(
        self: &Arc<Self>,
        webrtc_conn: WebRtcConnection,
        peer_connection: Arc<RTCPeerConnection>,
        target: ActrId,
    ) {
        let coord = Arc::downgrade(self);
        peer_connection.on_peer_connection_state_change(Box::new(
            move |state: RTCPeerConnectionState| {
                let coord = coord.clone();
                let target = target.clone();
                let webrtc_conn = webrtc_conn.clone();
                Box::pin(async move {
                    // First run the base WebRtcConnection cleanup.
                    webrtc_conn.handle_state_change(state).await;

                    tracing::info!(
                        "📡 PeerConnection state for {} -> {:?}",
                        target.serial_number,
                        state
                    );
                    if matches!(
                        state,
                        RTCPeerConnectionState::Disconnected | RTCPeerConnectionState::Failed
                    ) {
                        if let Some(c) = coord.upgrade() {
                            if let Err(e) = c.restart_ice(&target).await {
                                tracing::warn!(
                                    "⚠️ Failed to auto restart ICE to {}: {}",
                                    target.serial_number,
                                    e
                                );
                            }
                        }
                    }
                })
            },
        ));
    }
}
