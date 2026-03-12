//! WebRTC P2P Connection implementation

use crate::transport::connection_event::{ConnectionEvent, ConnectionState};
use crate::transport::session::ConnectionSession;
use crate::transport::{ConnType, DataLane, NetworkError, NetworkResult, WebRtcDataLane, WireHandle};
use crate::wire::webrtc::signaling::{HookCallback, HookEvent};
use actr_protocol::prost::Message;
use actr_protocol::{ActrId, PayloadType};
use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::sync::{RwLock, broadcast, mpsc};
use webrtc::data_channel::RTCDataChannel;
use webrtc::peer_connection::{RTCPeerConnection, peer_connection_state::RTCPeerConnectionState};
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;

/// Type alias for media track storage (track_id → (Track, Sender))
type MediaTracks = Arc<RwLock<HashMap<String, (Arc<TrackLocalStaticRTP>, Arc<RTCRtpSender>)>>>;

/// Type alias for lane cache array (PayloadType index → cached DataLane)
type LaneCache<const N: usize> = Arc<RwLock<[Option<Arc<dyn DataLane>>; N]>>;

/// WebRtcConnection - WebRTC P2P Connect
#[derive(Clone)]
pub struct WebRtcConnection {
    /// Peer ID for event identification
    peer_id: ActrId,

    /// underlying RTCPeerConnection
    peer_connection: Arc<RTCPeerConnection>,

    // TODO: useless property, remove this
    /// DataChannel Cache：PayloadType → DataChannel（4 types use DataChannel）
    /// index reference mapping：RpcReliable(0), RpcSignal(1), StreamReliable(2), StreamLatencyFirst(3)
    data_channels: Arc<RwLock<[Option<Arc<RTCDataChannel>>; 4]>>,

    /// MediaTrack Cache：track_id → (Track, RtpSender)
    media_tracks: MediaTracks,

    /// RTP sequence numbers per track (track_id → sequence_number)
    track_sequence_numbers: Arc<RwLock<HashMap<String, Arc<AtomicU16>>>>,

    /// RTP SSRC per track (track_id → ssrc)
    track_ssrcs: Arc<RwLock<HashMap<String, u32>>>,

    /// Lane Cache: PayloadType -> Lane (4 types use DataChannel)
    /// index mapping: RpcReliable(0), RpcSignal(1), StreamReliable(2), StreamLatencyFirst(3)
    /// MediaTrack not cached in array, uses HashMap
    lane_cache: LaneCache<4>,

    /// Event broadcaster for connection state changes
    event_tx: broadcast::Sender<ConnectionEvent>,

    hook_callback: Option<HookCallback>,

    /// Connection session (session_id + cancel_token + close-once)
    session: ConnectionSession,

    /// connection status (legacy, will be replaced by session.is_closed())
    connected: Arc<RwLock<bool>>,
}

impl std::fmt::Debug for WebRtcConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebRtcConnection")
            .field("peer_id", &self.peer_id)
            .field("peer_connection", &"<RTCPeerConnection>")
            .field("data_channels", &"<[Option<Arc<RTCDataChannel>>; 4]>")
            .field("media_tracks", &"<HashMap<String, Arc<Track>>>")
            .field("connected", &self.connected)
            .finish()
    }
}

impl WebRtcConnection {
    /// Create WebRtcConnection from RTCPeerConnection
    ///
    /// # Arguments
    /// - `peer_id`: Peer identity for event identification
    /// - `peer_connection`: Arc wrapped RTCPeerConnection
    /// - `event_tx`: Broadcast sender for connection events
    pub fn new(
        peer_id: ActrId,
        peer_connection: Arc<RTCPeerConnection>,
        event_tx: broadcast::Sender<ConnectionEvent>,
        hook_callback: Option<HookCallback>,
    ) -> Self {
        Self {
            peer_id,
            peer_connection,
            data_channels: Arc::new(RwLock::new([None, None, None, None])),
            media_tracks: Arc::new(RwLock::new(HashMap::new())),
            track_sequence_numbers: Arc::new(RwLock::new(HashMap::new())),
            track_ssrcs: Arc::new(RwLock::new(HashMap::new())),
            lane_cache: Arc::new(RwLock::new([None, None, None, None])),
            event_tx,
            hook_callback,
            session: ConnectionSession::new(),
            connected: Arc::new(RwLock::new(true)),
        }
    }

    /// Get peer ID
    pub fn peer_id(&self) -> &ActrId {
        &self.peer_id
    }

    /// Get session reference
    pub fn session(&self) -> &ConnectionSession {
        &self.session
    }

    /// Get session ID
    pub fn session_id(&self) -> u64 {
        self.session.session_id
    }

    /// Install a state-change handler on the underlying RTCPeerConnection.
    ///
    /// This keeps `connected` in sync with the WebRTC connection state and
    /// broadcasts state change events for upper layers to handle.
    pub(crate) async fn handle_state_change(&self, state: RTCPeerConnectionState) {
        // Guard: if session is cancelled, skip all side effects
        if self.session.is_cancelled() {
            tracing::debug!(
                "🚫 handle_state_change session {} cancelled, ignoring {:?}",
                self.session.session_id,
                state
            );
            return;
        }

        // Treat New/Connecting/Connected as "connected"; others as disconnected.
        let is_connected = matches!(
            state,
            RTCPeerConnectionState::New
                | RTCPeerConnectionState::Connecting
                | RTCPeerConnectionState::Connected
        );

        // Update flag and detect transitions from connected -> disconnected.
        let was_connected = {
            let mut flag = self.connected.write().await;
            let prev = *flag;
            *flag = is_connected;
            prev
        };

        // Convert WebRTC state to our ConnectionState
        let connection_state = match state {
            RTCPeerConnectionState::New => ConnectionState::New,
            RTCPeerConnectionState::Connecting => ConnectionState::Connecting,
            RTCPeerConnectionState::Connected => ConnectionState::Connected,
            RTCPeerConnectionState::Disconnected => ConnectionState::Disconnected,
            RTCPeerConnectionState::Failed => ConnectionState::Failed,
            RTCPeerConnectionState::Closed => ConnectionState::Closed,
            _ => ConnectionState::Closed, // Unspecified maps to Closed
        };

        tracing::info!(
            "🔄 WebRtcConnection peer state changed: {:?}, connected={}",
            state,
            is_connected
        );

        // Broadcast state change event for upper layers
        let _ = self.event_tx.send(ConnectionEvent::StateChanged {
            peer_id: self.peer_id.clone(),
            session_id: self.session.session_id,
            state: connection_state.clone(),
        });

        // Invoke hook synchronously (5s timeout to avoid blocking libwebrtc)
        if let Some(cb) = &self.hook_callback {
            let event = match connection_state {
                ConnectionState::Connecting => Some(HookEvent::WebRtcConnectStart {
                    peer_id: self.peer_id.clone(),
                }),
                ConnectionState::Connected => {
                    // Detect relay via ICE selected candidate pair
                    let sctp = self.peer_connection.sctp();
                    let dtls = sctp.transport();
                    let ice = dtls.ice_transport();
                    let relayed = match ice.get_selected_candidate_pair().await {
                        Some(pair) => pair.to_string().contains("relay"),
                        None => false,
                    };
                    tracing::debug!(
                        "WebRtcConnection peer connected: {:?}, relayed={}",
                        self.peer_id,
                        relayed
                    );
                    Some(HookEvent::WebRtcConnected {
                        peer_id: self.peer_id.clone(),
                        relayed,
                    })
                }
                ConnectionState::Disconnected
                | ConnectionState::Failed
                | ConnectionState::Closed => Some(HookEvent::WebRtcDisconnected {
                    peer_id: self.peer_id.clone(),
                }),
                _ => None,
            };
            if let Some(event) = event {
                if tokio::time::timeout(std::time::Duration::from_secs(5), cb(event))
                    .await
                    .is_err()
                {
                    tracing::warn!("⚠️ HookCallback timed out (5s) for peer {:?}", self.peer_id);
                }
            }
        }

        // For Closed state, proactively close the connection and let
        // `close()` perform all resource cleanup. Only trigger when we
        // transition from connected -> disconnected to avoid loops.
        if was_connected && matches!(state, RTCPeerConnectionState::Closed) {
            tracing::info!(
                "🔻 WebRtcConnection entering terminal state {:?}, calling close()",
                state
            );

            if let Err(e) = self.close().await {
                tracing::warn!("⚠️ WebRtcConnection::close() failed: {}", e);
            }
        }
    }

    /// Install a state-change handler on the underlying RTCPeerConnection.
    ///
    /// This keeps `connected` in sync with the WebRTC connection state and
    /// proactively closes the PeerConnection and clears internal caches when
    /// entering a terminal state (Disconnected/Failed/Closed).
    pub fn install_state_change_handler(&self) {
        let this = self.clone();

        self.peer_connection
            .on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
                let this = this.clone();

                Box::pin(async move {
                    this.handle_state_change(state).await;
                })
            }));
    }

    /// establish Connect（WebRTC Connect already alreadyvia signaling establish ， this in only is mark record ）
    pub async fn connect(&self) -> NetworkResult<()> {
        *self.connected.write().await = true;
        Ok(())
    }

    /// Broadcast DataChannel closed event
    ///
    /// Unlike the old AtomicBool-based notification, this broadcasts to all
    /// subscribers every time a DataChannel closes.
    fn notify_data_channel_closed(&self, payload_type: PayloadType) {
        //
        // The cleanup will be handled by the caller (close() or cleanup_cancelled_connection).
        // We only broadcast the event here to notify upper layers.
        let _ = self.event_tx.send(ConnectionEvent::DataChannelClosed {
            peer_id: self.peer_id.clone(),
            session_id: self.session.session_id,
            payload_type,
        });
    }

    /// Subscribe to connection events
    pub fn subscribe_events(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.event_tx.subscribe()
    }

    /// Checkwhether already Connect
    #[inline]
    pub fn is_connected(&self) -> bool {
        !self.session.is_closed()
    }

    /// Return a snapshot of the current DataChannel cache.
    ///
    /// Used by the coordinator to query `buffered_amount` on abnormal disconnect.
    pub async fn data_channels(&self) -> [Option<Arc<RTCDataChannel>>; 4] {
        self.data_channels.read().await.clone()
    }

    /// Check if any DataChannel is open
    pub async fn has_open_data_channel(&self) -> bool {
        use webrtc::data_channel::data_channel_state::RTCDataChannelState;

        let channels = self.data_channels.read().await;
        for channel in channels.iter().flatten() {
            if channel.ready_state() == RTCDataChannelState::Open {
                return true;
            }
        }
        false
    }

    /// Drain all open DataChannel send buffers before closing (graceful shutdown).
    ///
    /// Polls `buffered_amount()` up to 50 times with 100 ms intervals (max 5 s total).
    /// Logs a warning if the buffer is still non-zero when the timeout expires.
    async fn drain_data_channels(&self) {
        use webrtc::data_channel::data_channel_state::RTCDataChannelState;
        const MAX_POLLS: u32 = 50;
        const POLL_INTERVAL_MS: u64 = 100;

        // Snapshot open channels first, then release the lock before any async waits.
        let open_channels: Vec<(usize, Arc<RTCDataChannel>)> = {
            let channels = self.data_channels.read().await;
            channels
                .iter()
                .enumerate()
                .filter_map(|(idx, opt)| {
                    opt.as_ref().and_then(|ch| {
                        if ch.ready_state() == RTCDataChannelState::Open {
                            Some((idx, Arc::clone(ch)))
                        } else {
                            None
                        }
                    })
                })
                .collect()
        };

        for (idx, channel) in open_channels {
            let label = channel.label().to_owned();
            for attempt in 0..MAX_POLLS {
                let buffered = channel.buffered_amount().await;
                if buffered == 0 {
                    if attempt > 0 {
                        tracing::debug!(
                            peer_id = %self.peer_id.serial_number,
                            channel = %label,
                            channel_idx = idx,
                            attempts = attempt,
                            "DataChannel send buffer drained",
                        );
                    }
                    break;
                }

                if attempt == MAX_POLLS - 1 {
                    tracing::warn!(
                        peer_id = %self.peer_id.serial_number,
                        channel = %label,
                        channel_idx = idx,
                        buffered_bytes = buffered,
                        "DataChannel send buffer not fully drained before close; \
                         data may be lost for the peer",
                    );
                } else {
                    tokio::time::sleep(tokio::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
                }
            }
        }
    }

    /// Close connection and broadcast ConnectionClosed event
    ///
    /// This method is idempotent: only the first call performs the actual close.
    /// Subsequent calls return Ok(()) immediately.
    pub async fn close(&self) -> NetworkResult<()> {
        // Idempotent: only execute once per session
        if !self.session.try_close() {
            tracing::debug!(
                "🔒 [close] serial={} already closed (session_id={}), skipping",
                self.peer_id.serial_number,
                self.session.session_id
            );
            return Ok(());
        }

        // Cancel the session token — all callbacks holding a clone will notice
        self.session.cancel();

        tracing::debug!(
            "🔒 [close] serial={} session_id={} step 1: marking closed",
            self.peer_id.serial_number,
            self.session.session_id
        );
        *self.connected.write().await = false;

        // Drain DataChannel send buffers before closing (graceful shutdown).
        self.drain_data_channels().await;

        tracing::debug!(
            "🔒 [close] serial={} step 2: closing peer_connection",
            self.peer_id.serial_number
        );
        self.peer_connection.close().await?;

        // Clear each cache under a dedicated lock scope
        {
            let mut cache = self.lane_cache.write().await;
            *cache = [None, None, None, None];
        }
        {
            let mut channels = self.data_channels.write().await;
            *channels = [None, None, None, None];
        }
        {
            let mut tracks = self.media_tracks.write().await;
            tracks.clear();
        }
        {
            let mut seq_nums = self.track_sequence_numbers.write().await;
            seq_nums.clear();
        }
        {
            let mut ssrcs = self.track_ssrcs.write().await;
            ssrcs.clear();
        }

        // Broadcast ConnectionClosed event with real session_id
        let _ = self.event_tx.send(ConnectionEvent::ConnectionClosed {
            peer_id: self.peer_id.clone(),
            session_id: self.session.session_id,
        });

        tracing::info!(
            "🔌 WebRtcConnection closed for peer {:?} (session_id={})",
            self.peer_id,
            self.session.session_id
        );

        Ok(())
    }

    /// based on PayloadType configuration DataChannel
    fn get_data_channel_config(
        payload_type: &PayloadType,
    ) -> webrtc::data_channel::data_channel_init::RTCDataChannelInit {
        use webrtc::data_channel::data_channel_init::RTCDataChannelInit;

        match payload_type {
            PayloadType::StreamLatencyFirst => {
                // partial reliable transmission (low latency priority)
                RTCDataChannelInit {
                    ordered: Some(false),
                    max_retransmits: Some(3),
                    max_packet_life_time: None,
                    protocol: Some("".to_string()),
                    negotiated: None,
                }
            }
            _ => {
                // default reliable transmission
                RTCDataChannelInit {
                    ordered: Some(true),
                    max_retransmits: None,
                    max_packet_life_time: None,
                    protocol: Some("".to_string()),
                    negotiated: None,
                }
            }
        }
    }
}

impl WebRtcConnection {
    /// Get or create DataLane (with caching)
    pub async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        self.get_lane_internal(payload_type).await
    }

    /// Internal implementation of get_lane
    async fn get_lane_internal(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        // MediaTrack not supported in this method (needs stream_id)
        if payload_type == PayloadType::MediaRtp {
            return Err(NetworkError::NotImplemented(
                "MediaTrack Lane requires stream_id, use get_media_lane() instead".to_string(),
            ));
        }

        let idx = payload_type as usize;

        // 1. Check cache
        let mut need_recreate = false;
        {
            let cache = self.lane_cache.read().await;
            if let Some(lane) = &cache[idx] {
                // Check if the cached lane's transport is still healthy
                if !lane.is_healthy() {
                    tracing::warn!(
                        "Cached lane for {:?} is unhealthy, recreating",
                        payload_type,
                    );
                    need_recreate = true;
                } else {
                    tracing::debug!("Reuse cached DataLane: {:?}", payload_type);
                    return Ok(Arc::clone(lane));
                }
            }
        }

        if need_recreate {
            // Clear stale cache entries before recreating.
            let mut cache = self.lane_cache.write().await;
            cache[idx] = None;
            let mut channels = self.data_channels.write().await;
            channels[idx] = None;
        }

        // 2. Create new DataLane
        let lane = self.create_lane_internal(payload_type).await?;

        // 3. Cache
        {
            let mut cache = self.lane_cache.write().await;
            cache[idx] = Some(Arc::clone(&lane));
        }

        tracing::info!("✨ WebRtcConnection Createnew DataLane: {:?}", payload_type);

        Ok(lane)
    }

    /// Invalidate cached lane/DataChannel for given payload type.
    ///
    /// Used when the underlying DataChannel has transitioned to Closed and needs
    /// to be recreated on next `get_lane` call.
    pub async fn invalidate_lane(&self, payload_type: PayloadType) {
        self.invalidate_lane_internal(payload_type).await;
    }

    /// Internal implementation of invalidate_lane
    async fn invalidate_lane_internal(&self, payload_type: PayloadType) {
        let idx = payload_type as usize;
        let mut cache = self.lane_cache.write().await;
        cache[idx] = None;
        let mut channels = self.data_channels.write().await;
        channels[idx] = None;
    }

    /// Internal: Create DataChannel Lane (without cache)
    async fn create_lane_internal(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        // Checkwhetheras MediaTrack Type
        if payload_type == PayloadType::MediaRtp {
            return Err(NetworkError::NotImplemented(
                "MediaTrack Lane not implemented in this method".to_string(),
            ));
        }

        // Create new DataChannel
        let mut channels = self.data_channels.write().await;

        let label = payload_type.as_str_name();

        let dc_config = Self::get_data_channel_config(&payload_type);
        let data_channel = self
            .peer_connection
            .create_data_channel(label, Some(dc_config))
            .await?;

        // Register on_open callback to send DataChannelOpened event
        let event_tx_for_open = self.event_tx.clone();
        let peer_id_for_open = self.peer_id.clone();
        let session_id_for_open = self.session.session_id;
        let payload_type_for_open = payload_type;

        data_channel.on_open(Box::new(move || {
            let event_tx = event_tx_for_open.clone();
            let peer_id = peer_id_for_open.clone();
            let payload_type = payload_type_for_open;

            tracing::info!("🔄 WebRTC DataChannel opened: {:?}", payload_type);

            Box::pin(async move {
                let _ = event_tx.send(ConnectionEvent::DataChannelOpened {
                    peer_id,
                    session_id: session_id_for_open,
                    payload_type,
                });
                tracing::debug!("📣 DataChannelOpened event sent for {:?}", payload_type);
            })
        }));

        let channel_id = data_channel.id();
        let payload_type_for_error = payload_type;
        let label_for_error = label;
        data_channel.on_error(Box::new(move |error| {
            let payload_type = payload_type_for_error;
            let label = label_for_error;
            let channel_id = channel_id;
            tracing::warn!(
                "⚠️ WebRTC DataChannel error [{}] (payload_type={:?}, channel_id={}): {:?}",
                label,
                payload_type,
                channel_id,
                error
            );
            Box::pin(async move {})
        }));

        let session_for_close = self.session.clone();
        let lane_cache_for_close = self.lane_cache.clone();
        let data_channels_for_close = self.data_channels.clone();
        let event_tx_for_close = self.event_tx.clone();
        let peer_id_for_close = self.peer_id.clone();
        let sid_for_close = self.session.session_id;
        let payload_type_for_close = payload_type;
        let label_for_close = label;
        let channel_id_for_close = channel_id;
        let dc_for_close = Arc::clone(&data_channel);
        data_channel.on_close(Box::new(move || {
            let session = session_for_close.clone();
            let lane_cache = lane_cache_for_close.clone();
            let data_channels = data_channels_for_close.clone();
            let event_tx = event_tx_for_close.clone();
            let peer_id = peer_id_for_close.clone();
            let payload_type = payload_type_for_close;
            let label = label_for_close;
            let channel_id = channel_id_for_close;
            let dc = dc_for_close.clone();
            Box::pin(async move {
                // Guard: if session is cancelled (connection already cleaned up),
                // skip all side effects to avoid corrupting a new connection
                if session.is_cancelled() {
                    tracing::debug!(
                        "🚫 DC.on_close session {} cancelled, ignoring for {:?}",
                        sid_for_close,
                        payload_type
                    );
                    return;
                }

                // Query buffered_amount at the moment of close to surface potential data loss.
                let buffered = dc.buffered_amount().await;
                if buffered > 0 {
                    tracing::warn!(
                        channel = %label,
                        channel_id = channel_id,
                        payload_type = ?payload_type,
                        buffered_bytes = buffered,
                        "DataChannel closed with non-empty send buffer",
                    );
                } else {
                    tracing::warn!(
                        "DataChannel closed [{}] (payload_type={:?}, channel_id={})",
                        label,
                        payload_type,
                        channel_id,
                    );
                }
                // Invalidate cached lane when DataChannel closes
                let idx = payload_type as usize;
                {
                    let mut cache = lane_cache.write().await;
                    cache[idx] = None;
                }
                {
                    let mut channels = data_channels.write().await;
                    channels[idx] = None;
                }
                // Broadcast DataChannelClosed event
                let _ = event_tx.send(ConnectionEvent::DataChannelClosed {
                    peer_id,
                    session_id: sid_for_close,
                    payload_type,
                });
            })
        }));

        // CreateReceive channel （using Bytes）
        let (tx, rx) = mpsc::channel(100);

        // Set onmessage return adjust
        let tx_clone = tx.clone();
        data_channel.on_message(Box::new(
            move |msg: webrtc::data_channel::data_channel_message::DataChannelMessage| {
                // zero-copy： directly using msg.data (Bytes)
                let data = msg.data;
                tracing::debug!("🔄 WebRTC DataChannel message received1111: {:?}", data);
                let tx = tx_clone.clone();
                Box::pin(async move {
                    if let Err(e) = tx.send(data).await {
                        tracing::warn!("❌ WebRTC DataChannel messageSend to Lane failure: {}", e);
                    }
                })
            },
        ));

        // Cache DataChannel（ index reference directly using PayloadType value ）
        let idx = payload_type as usize;
        channels[idx] = Some(Arc::clone(&data_channel));

        // Returns Lane
        Ok(Arc::new(WebRtcDataLane::new(data_channel, rx)))
    }

    /// Add media track to PeerConnection
    ///
    /// # Arguments
    /// - `track_id`: Unique track identifier
    /// - `codec`: Codec name (e.g., "H264", "VP8", "opus")
    /// - `media_type`: "video" or "audio"
    ///
    /// # Returns
    /// Reference to the created TrackLocalStaticRTP
    ///
    /// # Note
    /// Must be called BEFORE create_offer/create_answer for track to appear in SDP
    pub async fn add_media_track(
        &self,
        track_id: String,
        codec: &str,
        media_type: &str,
    ) -> NetworkResult<Arc<TrackLocalStaticRTP>> {
        use webrtc::api::media_engine::MIME_TYPE_H264;
        use webrtc::api::media_engine::MIME_TYPE_OPUS;
        use webrtc::api::media_engine::MIME_TYPE_VP8;
        use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;

        // Reuse existing track so repeated start/stop flows can safely retry.
        if let Some((track, _sender)) = self.media_tracks.read().await.get(&track_id).cloned() {
            tracing::info!("♻️ Reusing existing media track: {}", track_id);
            return Ok(track);
        }

        // Determine MIME type based on codec and media_type
        let mime_type = match (media_type, codec.to_uppercase().as_str()) {
            ("video", "H264") => MIME_TYPE_H264,
            ("video", "VP8") => MIME_TYPE_VP8,
            ("audio", "OPUS") => MIME_TYPE_OPUS,
            _ => {
                return Err(NetworkError::WebRtcError(format!(
                    "Unsupported codec: {codec} for {media_type}"
                )));
            }
        };

        // Create TrackLocalStaticRTP
        let track = Arc::new(TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: mime_type.to_string(),
                ..Default::default()
            },
            track_id.clone(),
            format!("actr-{media_type}"), // stream_id
        ));

        // Add track to PeerConnection
        let rtp_sender =
            self.peer_connection
                .add_track(Arc::clone(&track)
                    as Arc<dyn webrtc::track::track_local::TrackLocal + Send + Sync>)
                .await?;

        // Cache track and sender
        let mut tracks = self.media_tracks.write().await;
        tracks.insert(track_id.clone(), (Arc::clone(&track), rtp_sender));

        // Initialize sequence number for this track
        let mut seq_nums = self.track_sequence_numbers.write().await;
        seq_nums.insert(track_id.clone(), Arc::new(AtomicU16::new(0)));

        // Generate unique SSRC for this track (random u32)
        let ssrc = rand::random::<u32>();
        let mut ssrcs = self.track_ssrcs.write().await;
        ssrcs.insert(track_id.clone(), ssrc);

        tracing::info!(
            "✨ Added media track: id={}, codec={}, type={}, ssrc=0x{:08x}",
            track_id,
            codec,
            media_type,
            ssrc
        );

        Ok(track)
    }

    /// Remove a media track and its RTP sender from the PeerConnection
    pub async fn remove_media_track(&self, track_id: &str) -> NetworkResult<()> {
        let removed = self.media_tracks.write().await.remove(track_id);
        if let Some((_track, rtp_sender)) = removed {
            self.peer_connection.remove_track(&rtp_sender).await?;
            self.track_sequence_numbers.write().await.remove(track_id);
            self.track_ssrcs.write().await.remove(track_id);
            tracing::info!("🗑️ Removed media track: {}", track_id);
        }
        Ok(())
    }

    /// Get existing media track by ID
    pub async fn get_media_track(&self, track_id: &str) -> Option<Arc<TrackLocalStaticRTP>> {
        let tracks = self.media_tracks.read().await;
        tracks
            .get(track_id)
            .map(|(track, _sender)| Arc::clone(track))
    }

    /// Get next RTP sequence number for track (atomically increments)
    ///
    /// # Arguments
    /// - `track_id`: Track identifier
    ///
    /// # Returns
    /// Next sequence number (wraps at 65535)
    pub async fn next_sequence_number(&self, track_id: &str) -> Option<u16> {
        let seq_nums = self.track_sequence_numbers.read().await;
        seq_nums
            .get(track_id)
            .map(|atomic_seq| atomic_seq.fetch_add(1, Ordering::SeqCst))
    }

    /// Get SSRC for track
    ///
    /// # Arguments
    /// - `track_id`: Track identifier
    ///
    /// # Returns
    /// SSRC value for this track
    pub async fn get_ssrc(&self, track_id: &str) -> Option<u32> {
        let ssrcs = self.track_ssrcs.read().await;
        ssrcs.get(track_id).copied()
    }

    /// GetorCreate MediaTrack Lane（ carry Cache）
    ///
    /// # Arguments
    /// - `_stream_id`: Media stream ID
    ///
    /// backwardaftercompatible hold Method：create_lane adjust usage get_lane
    pub async fn create_lane(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        self.get_lane(payload_type).await
    }

    /// Register received DataChannel (for passive side)
    ///
    /// When receiving an Offer, the passive side should register DataChannels
    /// received via on_data_channel callback instead of creating new ones.
    pub async fn register_received_data_channel(
        &self,
        data_channel: Arc<RTCDataChannel>,
        payload_type: PayloadType,
        message_tx: mpsc::UnboundedSender<(Vec<u8>, Bytes, PayloadType)>,
    ) -> NetworkResult<Arc<dyn DataLane>> {
        // Check if it's MediaTrack type
        if payload_type == PayloadType::MediaRtp {
            return Err(NetworkError::NotImplemented(
                "MediaTrack Lane not supported in this method".to_string(),
            ));
        }

        let idx = payload_type as usize;
        tracing::debug!(
            "🔄 WebRTC DataChannel registered received: {:?}, idx={}",
            payload_type,
            idx
        );
        let label = format!("{payload_type:?}");

        // Register on_open callback to send DataChannelOpened event
        let event_tx_for_open = self.event_tx.clone();
        let peer_id_for_open = self.peer_id.clone();
        let session_id_for_open = self.session.session_id;
        let payload_type_for_open = payload_type;

        data_channel.on_open(Box::new(move || {
            let event_tx = event_tx_for_open.clone();
            let peer_id = peer_id_for_open.clone();
            let payload_type = payload_type_for_open;

            tracing::info!(
                "🔄 WebRTC DataChannel opened (received): {:?}",
                payload_type
            );

            Box::pin(async move {
                let _ = event_tx.send(ConnectionEvent::DataChannelOpened {
                    peer_id,
                    session_id: session_id_for_open,
                    payload_type,
                });
                tracing::debug!("📣 DataChannelOpened event sent for {:?}", payload_type);
            })
        }));

        // Set error handler
        let payload_type_for_error = payload_type;
        let label_for_error = label.clone();
        data_channel.on_error(Box::new(move |error| {
            let payload_type = payload_type_for_error;
            let label = label_for_error.clone();
            tracing::warn!(
                "⚠️ WebRTC DataChannel error [{}] (payload_type={:?} ): {:?}",
                label,
                payload_type,
                error
            );
            Box::pin(async move {})
        }));

        // Set close handler
        let this_for_close = self.clone();
        let payload_type_for_close = payload_type;
        let label_for_close = label.clone();
        let dc_for_close = Arc::clone(&data_channel);

        data_channel.on_close(Box::new(move || {
            let this = this_for_close.clone();
            let payload_type = payload_type_for_close;
            let label = label_for_close.clone();
            let dc = dc_for_close.clone();

            Box::pin(async move {
                // Query buffered_amount at the moment of close to surface potential data loss.
                let buffered = dc.buffered_amount().await;
                if buffered > 0 {
                    tracing::warn!(
                        peer_id = %this.peer_id.serial_number,
                        channel = %label,
                        payload_type = ?payload_type,
                        buffered_bytes = buffered,
                        "DataChannel (received) closed with non-empty send buffer; \
                         buffered data was likely not delivered to peer",
                    );
                } else {
                    tracing::warn!(
                        "DataChannel (received) closed [{}] (payload_type={:?})",
                        label,
                        payload_type,
                    );
                }
                // Invalidate cached lane when DataChannel closes
                this.invalidate_lane(payload_type).await;
                // Broadcast DataChannelClosed event (sync, no await needed)
                this.notify_data_channel_closed(payload_type);
            })
        }));

        // Create receive channel
        let (tx, rx) = mpsc::channel(100);

        // Set on_message callback
        let tx_clone = tx.clone();
        data_channel.on_message(Box::new(
            move |msg: webrtc::data_channel::data_channel_message::DataChannelMessage| {
                let data = msg.data;
                let tx = tx_clone.clone();
                Box::pin(async move {
                    if let Err(e) = tx.send(data).await {
                        tracing::warn!("❌ WebRTC DataChannel message send to Lane failed: {}", e);
                    }
                })
            },
        ));

        // Cache DataChannel
        {
            let mut channels = self.data_channels.write().await;
            channels[idx] = Some(Arc::clone(&data_channel));
        }

        // Create and cache Lane
        let lane: Arc<dyn DataLane> = Arc::new(WebRtcDataLane::new(data_channel, rx));
        {
            let mut cache = self.lane_cache.write().await;
            cache[idx] = Some(Arc::clone(&lane));
        }

        tracing::info!(
            "✨ WebRtcConnection registered received DataChannel: {:?}",
            payload_type
        );
        let peer_id_clone = self.peer_id.clone();
        let lane_clone = lane.clone();
        tokio::spawn(async move {
            // Continuously receive messages
            loop {
                match lane_clone.recv().await {
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
                        if let Err(e) = message_tx.send((peer_id_bytes, data, payload_type)) {
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
        });

        Ok(lane)
    }
}

#[async_trait]
impl WireHandle for WebRtcConnection {
    fn connection_type(&self) -> ConnType {
        ConnType::WebRTC
    }

    fn priority(&self) -> u8 {
        1 // WebRTC has higher priority
    }

    async fn connect(&self) -> NetworkResult<()> {
        Self::connect(self).await
    }

    fn is_connected(&self) -> bool {
        Self::is_connected(self)
    }

    async fn close(&self) -> NetworkResult<()> {
        Self::close(self).await
    }

    async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        self.get_lane_internal(payload_type).await
    }

    async fn invalidate_lane(&self, payload_type: PayloadType) {
        self.invalidate_lane_internal(payload_type).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast;
    use webrtc::api::APIBuilder;
    use webrtc::peer_connection::configuration::RTCConfiguration;
    /// Helper: create a WebRtcConnection for testing
    async fn create_test_connection() -> WebRtcConnection {
        let api = APIBuilder::new().build();
        let peer_connection = api
            .new_peer_connection(RTCConfiguration::default())
            .await
            .expect("Failed to create RTCPeerConnection");
        let (event_tx, _) = broadcast::channel(16);
        let peer_id = ActrId {
            realm: actr_protocol::Realm { realm_id: 1 },
            serial_number: 42,
            r#type: actr_protocol::ActrType {
                manufacturer: "test".to_string(),
                name: "node".to_string(),
                version: "v1".to_string(),
            },
        };
        WebRtcConnection::new(peer_id, Arc::new(peer_connection), event_tx, None)
    }

    /// Test: multiple tasks calling close() concurrently do not deadlock
    ///
    /// close() acquires write locks on multiple RwLocks sequentially (connected, data_channels,
    /// media_tracks, track_sequence_numbers, track_ssrcs, lane_cache).
    /// If two close() calls acquire them in different order or wait while holding locks, deadlock occurs.
    /// This test detects deadlock via timeout.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_close_no_deadlock() {
        let conn = create_test_connection().await;
        let num_tasks = 10;
        let mut handles = Vec::with_capacity(num_tasks);

        for i in 0..num_tasks {
            let conn = conn.clone();
            handles.push(tokio::spawn(async move {
                let result = conn.close().await;
                tracing::info!("Task {} close result: {:?}", i, result.is_ok());
                result
            }));
        }

        // Detect deadlock via timeout: no deadlock if all tasks finish within 2 seconds
        let all_tasks = futures_util::future::join_all(handles);
        let result = tokio::time::timeout(Duration::from_secs(2), all_tasks).await;

        match result {
            Ok(results) => {
                // All tasks should succeed (first close actually closes, subsequent ones may encounter already-closed connection)
                let completed = results.iter().filter(|r| r.is_ok()).count();
                assert_eq!(
                    completed, num_tasks,
                    "all {} tasks should complete, actually completed {}",
                    num_tasks, completed
                );
            }
            Err(_) => {
                panic!(
                    "deadlock detected: {} concurrent close() calls did not finish within 2 seconds, possible deadlock!",
                    num_tasks
                );
            }
        }
    }

    /// Test: close() with concurrent read operations does not deadlock
    ///
    /// Scenario: some tasks continuously read is_connected() / has_open_data_channel(),
    /// while others call close(). RwLock read-write contention should not cause deadlock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_close_with_concurrent_reads_no_deadlock() {
        let conn: WebRtcConnection = create_test_connection().await;
        let mut handles = Vec::new();

        // Spawn 5 reader tasks that continuously read connection state
        for i in 0..5 {
            let conn = conn.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..20 {
                    // Use async read instead of blocking_read (is_connected) to avoid async context issues
                    let _ = *conn.connected.read().await;
                    let _ = conn.has_open_data_channel().await;
                    tokio::task::yield_now().await;
                }
                tracing::info!("Reader task {} done", i);
            }));
        }

        // Spawn 5 close tasks
        for i in 0..5 {
            let conn = conn.clone();
            handles.push(tokio::spawn(async move {
                let result = conn.close().await;
                tracing::info!("Close task {} result: {:?}", i, result.is_ok());
            }));
        }

        let all_tasks = futures_util::future::join_all(handles);
        let result = tokio::time::timeout(Duration::from_secs(2), all_tasks).await;

        match result {
            Ok(results) => {
                let completed = results.iter().filter(|r| r.is_ok()).count();
                assert_eq!(completed, 10, "all 10 tasks should complete");
            }
            Err(_) => {
                panic!(
                    "deadlock detected: close() with concurrent reads did not finish within 2 seconds, possible deadlock!"
                );
            }
        }
    }

    /// Test: close() with concurrent handle_state_change() does not deadlock
    ///
    /// Real-world reproduction: after ICE restart failure, cleanup_cancelled_connection calls
    /// peer_connection.close(), which triggers a state_change callback invoking handle_state_change(Closed),
    /// and handle_state_change(Closed) internally calls self.close() again.
    /// This simulates the actual 3-way concurrent close race.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_close_with_handle_state_change_no_deadlock() {
        let conn = create_test_connection().await;
        let mut handles = Vec::new();

        // Simulate cleanup_cancelled_connection path: call close() directly
        {
            let conn = conn.clone();
            handles.push(tokio::spawn(async move {
                let _ = conn.close().await;
                tracing::info!("Direct close() done");
            }));
        }

        // Simulate state_change callback path: handle_state_change(Closed)
        // handle_state_change internally also calls close() when was_connected && Closed
        {
            let conn = conn.clone();
            handles.push(tokio::spawn(async move {
                conn.handle_state_change(
                    webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed,
                )
                .await;
                tracing::info!("handle_state_change(Closed) done");
            }));
        }

        // Simulate event listener path: call close() after receiving StateChanged(Closed)
        {
            let conn = conn.clone();
            handles.push(tokio::spawn(async move {
                let _ = conn.close().await;
                tracing::info!("Event listener close() done");
            }));
        }

        let all_tasks = futures_util::future::join_all(handles);
        let result = tokio::time::timeout(Duration::from_secs(2), all_tasks).await;

        match result {
            Ok(results) => {
                let completed = results.iter().filter(|r| r.is_ok()).count();
                assert_eq!(completed, 3, "all 3 tasks should complete");
            }
            Err(_) => {
                panic!(
                    "deadlock detected: close() with concurrent handle_state_change did not finish within 2 seconds, \
                     possible deadlock! This reproduces the 3-way close race after ICE restart failure."
                );
            }
        }
    }

    /// Test: stress test with many concurrent close() calls
    ///
    /// Uses more concurrent tasks to increase lock contention probability, making potential deadlocks easier to expose.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_stress_concurrent_close() {
        let conn = create_test_connection().await;
        let num_tasks = 50;
        let mut handles = Vec::with_capacity(num_tasks);

        for i in 0..num_tasks {
            let conn = conn.clone();
            handles.push(tokio::spawn(async move {
                // Mix close and read operations to increase lock contention
                if i % 3 == 0 {
                    let _ = *conn.connected.read().await;
                }
                if i % 5 == 0 {
                    let _ = conn.has_open_data_channel().await;
                }
                let _ = conn.close().await;
            }));
        }

        let all_tasks = futures_util::future::join_all(handles);
        let result = tokio::time::timeout(Duration::from_secs(3), all_tasks).await;

        match result {
            Ok(results) => {
                let completed = results.iter().filter(|r| r.is_ok()).count();
                assert_eq!(
                    completed, num_tasks,
                    "all {} stress test tasks should complete",
                    num_tasks
                );
                // Verify final state: connection should be closed
                assert!(
                    !*conn.connected.read().await,
                    "connected should be false after close()"
                );
            }
            Err(_) => {
                panic!(
                    "stress test deadlock detected: {} concurrent close() calls did not finish within 3 seconds, possible deadlock!",
                    num_tasks
                );
            }
        }
    }

    /// Regression test: close() with concurrent invalidate_lane() does not block due to lock order inversion
    ///
    /// This test simulates a historically reproduced sequence:
    /// - close() cleans up cache;
    /// - invalidate_lane() fires concurrently (lane_cache -> data_channels).
    /// After fix, both should complete within the timeout window without waiting on each other.
    #[tokio::test]
    async fn repro_close_blocked_by_lock_order_inversion() {
        use tokio::time::{Duration, sleep};

        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_test_writer()
            .try_init();

        let conn = create_test_connection().await;
        let payload_type = PayloadType::RpcReliable;

        // First create a DataChannel lane to ensure related caches and callback paths are established.
        let _ = conn
            .get_lane(payload_type)
            .await
            .expect("failed to create lane for repro");

        // Artificially stall close(): hold media_tracks first to ensure a concurrency window
        // between close and invalidate_lane (historically this triggered lock order contention).
        let media_tracks_guard = conn.media_tracks.write().await;

        let conn_for_close = conn.clone();
        let mut close_task = tokio::spawn(async move { conn_for_close.close().await });

        // Give close a brief moment to enter the cleanup path.
        sleep(Duration::from_millis(50)).await;

        // Trigger invalidate_lane concurrently (historically this would contend with close on lock order).
        let conn_for_invalidate = conn.clone();
        let mut invalidate_task = tokio::spawn(async move {
            conn_for_invalidate.invalidate_lane(payload_type).await;
        });

        sleep(Duration::from_millis(50)).await;

        // Release media_tracks to let close finish remaining cleanup.
        drop(media_tracks_guard);

        let result = tokio::time::timeout(Duration::from_millis(3000), async {
            let close_res = (&mut close_task).await;
            let invalidate_res = (&mut invalidate_task).await;
            (close_res, invalidate_res)
        })
        .await;

        match result {
            Ok((close_res, invalidate_res)) => {
                assert!(close_res.is_ok(), "close task panicked unexpectedly");
                assert!(
                    invalidate_res.is_ok(),
                    "invalidate task panicked unexpectedly"
                );
            }
            Err(_) => {
                close_task.abort();
                invalidate_task.abort();
                let _ = close_task.await;
                let _ = invalidate_task.await;
                panic!("close()/invalidate_lane() should not block after lock-order fix");
            }
        }
    }
}
