//! WebRTC P2P Connection implementation

use crate::transport::DataLane;
use crate::transport::connection_event::{ConnectionEvent, ConnectionState};
use crate::transport::{NetworkError, NetworkResult};
use actr_protocol::{ActrId, PayloadType};
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

    /// Lane Cache：PayloadType → Lane（ merely 3 solely proportion Type）
    /// index reference mapping：RpcReliable(0), RpcSignal(1), StreamReliable(2), StreamLatencyFirst(3)
    /// MediaTrack not Cachein array in ，using HashMap
    lane_cache: Arc<RwLock<[Option<DataLane>; 4]>>,

    /// Event broadcaster for connection state changes
    event_tx: broadcast::Sender<ConnectionEvent>,

    /// connection status
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
            connected: Arc::new(RwLock::new(true)),
        }
    }

    /// Get peer ID
    pub fn peer_id(&self) -> &ActrId {
        &self.peer_id
    }

    /// Install a state-change handler on the underlying RTCPeerConnection.
    ///
    /// This keeps `connected` in sync with the WebRTC connection state and
    /// broadcasts state change events for upper layers to handle.
    pub(crate) async fn handle_state_change(&self, state: RTCPeerConnectionState) {
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
            state: connection_state.clone(),
        });

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
        *self.connected.blocking_read()
    }

    /// Close connection and broadcast ConnectionClosed event
    pub async fn close(&self) -> NetworkResult<()> {
        *self.connected.write().await = false;
        self.peer_connection.close().await?;

        // clear blank DataChannel Cache
        let mut channels = self.data_channels.write().await;
        *channels = [None, None, None, None];

        // clear blank MediaTrack Cache
        let mut tracks = self.media_tracks.write().await;
        tracks.clear();

        // clear blank sequence number cache
        let mut seq_nums = self.track_sequence_numbers.write().await;
        seq_nums.clear();

        // clear blank SSRC cache
        let mut ssrcs = self.track_ssrcs.write().await;
        ssrcs.clear();

        // clear blank Lane Cache
        let mut cache = self.lane_cache.write().await;
        *cache = [None, None, None, None];

        // Broadcast ConnectionClosed event
        let _ = self.event_tx.send(ConnectionEvent::ConnectionClosed {
            peer_id: self.peer_id.clone(),
        });

        tracing::info!("🔌 WebRtcConnection closed for peer {:?}", self.peer_id);
        Ok(())
    }

    /// based on PayloadType configuration DataChannel
    fn get_data_channel_config(
        payload_type: PayloadType,
    ) -> webrtc::data_channel::data_channel_init::RTCDataChannelInit {
        use webrtc::data_channel::data_channel_init::RTCDataChannelInit;

        // Use negotiated DataChannel with fixed IDs based on PayloadType
        // This allows both sides to create the same channel without on_data_channel callback
        let channel_id = payload_type as u16;

        // TODO: remove negotiated flag to use auto-negotiation
        match payload_type {
            PayloadType::RpcSignal | PayloadType::RpcReliable => {
                // reliable ordered transmission
                RTCDataChannelInit {
                    ordered: Some(true),
                    max_retransmits: None,
                    max_packet_life_time: None,
                    protocol: Some("".to_string()),
                    negotiated: Some(channel_id),
                }
            }
            PayloadType::StreamLatencyFirst => {
                // partial reliable transmission (low latency priority)
                // NOTE: WebRTC spec forbids setting both max_retransmits and max_packet_life_time.
                RTCDataChannelInit {
                    ordered: Some(false),
                    max_retransmits: Some(3),
                    max_packet_life_time: None,
                    protocol: Some("".to_string()),
                    negotiated: Some(channel_id),
                }
            }
            _ => {
                // default reliable transmission
                RTCDataChannelInit {
                    ordered: Some(true),
                    max_retransmits: None,
                    max_packet_life_time: None,
                    protocol: Some("".to_string()),
                    negotiated: Some(channel_id),
                }
            }
        }
    }
}

impl WebRtcConnection {
    /// GetorCreate DataLane（ carry Cache）
    pub async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
        // MediaTrack not Supportin this Method in Create（need stream_id）
        if payload_type == PayloadType::MediaRtp {
            return Err(NetworkError::NotImplemented(
                "MediaTrack Lane requires stream_id, use get_media_lane() instead".to_string(),
            ));
        }

        let idx = payload_type as usize;

        // 1. CheckCache
        let mut need_recreate = false;
        {
            let cache = self.lane_cache.read().await;
            if let Some(lane) = &cache[idx] {
                // If the cached lane is backed by a DataChannel, ensure it is still open.
                if let DataLane::WebRtcDataChannel { data_channel, .. } = lane {
                    use webrtc::data_channel::data_channel_state::RTCDataChannelState;
                    let state = data_channel.ready_state();
                    if matches!(
                        state,
                        RTCDataChannelState::Closed | RTCDataChannelState::Closing
                    ) {
                        tracing::warn!(
                            "♻️ Cached DataChannel for {:?} is {:?}, recreating lane",
                            payload_type,
                            state
                        );
                        need_recreate = true;
                    } else {
                        tracing::debug!("📦 ReuseCache DataLane: {:?}", payload_type);
                        return Ok(lane.clone());
                    }
                } else {
                    tracing::debug!("📦 ReuseCache DataLane: {:?}", payload_type);
                    return Ok(lane.clone());
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

        // 2. Createnew DataLane
        let lane = self.create_lane_internal(payload_type).await?;

        // 3. Cache
        {
            let mut cache = self.lane_cache.write().await;
            cache[idx] = Some(lane.clone());
        }

        tracing::info!("✨ WebRtcConnection Createnew DataLane: {:?}", payload_type);

        Ok(lane)
    }

    /// Invalidate cached lane/DataChannel for given payload type.
    ///
    /// Used when the underlying DataChannel has transitioned to Closed and needs
    /// to be recreated on next `get_lane` call.
    pub async fn invalidate_lane(&self, payload_type: PayloadType) {
        let idx = payload_type as usize;
        let mut cache = self.lane_cache.write().await;
        cache[idx] = None;
        let mut channels = self.data_channels.write().await;
        channels[idx] = None;
    }

    /// inner part Method：Create DataChannel Lane（ not carry Cache）
    async fn create_lane_internal(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
        // Checkwhetheras MediaTrack Type
        if payload_type == PayloadType::MediaRtp {
            return Err(NetworkError::NotImplemented(
                "MediaTrack Lane not implemented in this method".to_string(),
            ));
        }

        // Create new DataChannel
        let mut channels = self.data_channels.write().await;

        let label = format!("{payload_type:?}");
        let dc_config = Self::get_data_channel_config(payload_type);

        let data_channel = self
            .peer_connection
            .create_data_channel(&label, Some(dc_config))
            .await?;

        data_channel.on_open(Box::new(move || {
            tracing::info!("🔄 WebRTC DataChannel opened: {:?}", payload_type);
            Box::pin(async move {})
        }));

        let channel_id = data_channel.id();
        let payload_type_for_error = payload_type;
        let label_for_error = label.clone();
        data_channel.on_error(Box::new(move |error| {
            let payload_type = payload_type_for_error;
            let label = label_for_error.clone();
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

        let this_for_close = self.clone();
        let payload_type_for_close = payload_type;
        let label_for_close = label.clone();
        let channel_id_for_close = channel_id;
        data_channel.on_close(Box::new(move || {
            let this = this_for_close.clone();
            let payload_type = payload_type_for_close;
            let label = label_for_close.clone();
            let channel_id = channel_id_for_close;
            Box::pin(async move {
                tracing::warn!(
                    "⚠️ WebRTC DataChannel closed [{}] (payload_type={:?}, channel_id={})",
                    label,
                    payload_type,
                    channel_id
                );
                // Invalidate cached lane when DataChannel closes
                this.invalidate_lane(payload_type).await;
                // Broadcast DataChannelClosed event (sync, no await needed)
                this.notify_data_channel_closed(payload_type);
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
        Ok(DataLane::webrtc_data_channel(data_channel, rx))
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
    pub async fn create_lane(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
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
    ) -> NetworkResult<DataLane> {
        // Check if it's MediaTrack type
        if payload_type == PayloadType::MediaRtp {
            return Err(NetworkError::NotImplemented(
                "MediaTrack Lane not supported in this method".to_string(),
            ));
        }

        let idx = payload_type as usize;
        let channel_id = data_channel.id();
        let label = format!("{payload_type:?}");

        // Set error handler
        let payload_type_for_error = payload_type;
        let label_for_error = label.clone();
        let channel_id_for_error = channel_id;
        data_channel.on_error(Box::new(move |error| {
            let payload_type = payload_type_for_error;
            let label = label_for_error.clone();
            let channel_id = channel_id_for_error;
            tracing::warn!(
                "⚠️ WebRTC DataChannel error [{}] (payload_type={:?}, channel_id={}): {:?}",
                label,
                payload_type,
                channel_id,
                error
            );
            Box::pin(async move {})
        }));

        // Set close handler
        let this_for_close = self.clone();
        let payload_type_for_close = payload_type;
        let label_for_close = label.clone();
        let channel_id_for_close = channel_id;
        data_channel.on_close(Box::new(move || {
            let this = this_for_close.clone();
            let payload_type = payload_type_for_close;
            let label = label_for_close.clone();
            let channel_id = channel_id_for_close;
            Box::pin(async move {
                tracing::warn!(
                    "⚠️ WebRTC DataChannel closed [{}] (payload_type={:?}, channel_id={})",
                    label,
                    payload_type,
                    channel_id
                );
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
        let lane = DataLane::webrtc_data_channel(data_channel, rx);
        {
            let mut cache = self.lane_cache.write().await;
            cache[idx] = Some(lane.clone());
        }

        tracing::info!(
            "✨ WebRtcConnection registered received DataChannel: {:?}",
            payload_type
        );

        Ok(lane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note：WebRTC gather integrate measure try needCompletesignaling stream process ， this in only do solely element measure try

    #[test]
    fn test_data_channel_config() {
        let config = WebRtcConnection::get_data_channel_config(PayloadType::RpcReliable);
        assert_eq!(config.ordered, Some(true));

        let config = WebRtcConnection::get_data_channel_config(PayloadType::StreamLatencyFirst);
        assert_eq!(config.ordered, Some(false));
        assert_eq!(config.max_retransmits, Some(3));
    }
}
