//! WebRTC P2P Connection implementation

use crate::transport::DataLane;
use crate::transport::{NetworkError, NetworkResult};
use actr_protocol::PayloadType;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::sync::{RwLock, mpsc};
use webrtc::data_channel::RTCDataChannel;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;

/// Type alias for media track storage (track_id → (Track, Sender))
type MediaTracks = Arc<RwLock<HashMap<String, (Arc<TrackLocalStaticRTP>, Arc<RTCRtpSender>)>>>;

/// WebRtcConnection - WebRTC P2P Connect
#[derive(Clone)]
pub struct WebRtcConnection {
    /// underlying RTCPeerConnection
    peer_connection: Arc<RTCPeerConnection>,

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

    /// connection status
    connected: Arc<RwLock<bool>>,
}

impl std::fmt::Debug for WebRtcConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebRtcConnection")
            .field("peer_connection", &"<RTCPeerConnection>")
            .field("data_channels", &"<[Option<Arc<RTCDataChannel>>; 4]>")
            .field("media_tracks", &"<HashMap<String, Arc<Track>>>")
            .field("connected", &self.connected)
            .finish()
    }
}

impl WebRtcConnection {
    /// from RTCPeerConnection CreateConnect
    ///
    /// # Arguments
    /// - `peer_connection`: Arc package pack 's RTCPeerConnection
    pub fn new(peer_connection: Arc<RTCPeerConnection>) -> Self {
        Self {
            peer_connection,
            data_channels: Arc::new(RwLock::new([None, None, None, None])),
            media_tracks: Arc::new(RwLock::new(HashMap::new())),
            track_sequence_numbers: Arc::new(RwLock::new(HashMap::new())),
            track_ssrcs: Arc::new(RwLock::new(HashMap::new())),
            lane_cache: Arc::new(RwLock::new([None, None, None, None])),
            connected: Arc::new(RwLock::new(true)),
        }
    }

    /// establish Connect（WebRTC Connect already alreadyvia signaling establish ， this in only is mark record ）
    pub async fn connect(&self) -> NetworkResult<()> {
        *self.connected.write().await = true;
        Ok(())
    }

    /// Checkwhether already Connect
    #[inline]
    pub fn is_connected(&self) -> bool {
        *self.connected.blocking_read()
    }

    /// CloseConnect
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

        tracing::info!("🔌 WebRtcConnection already Close");
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
                RTCDataChannelInit {
                    ordered: Some(false),
                    max_retransmits: Some(3),
                    max_packet_life_time: Some(100),
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
        {
            let cache = self.lane_cache.read().await;
            if let Some(lane) = &cache[idx] {
                tracing::debug!("📦 ReuseCache DataLane: {:?}", payload_type);
                return Ok(lane.clone());
            }
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

        // CreateReceive channel （using Bytes）
        let (tx, rx) = mpsc::channel(100);

        // Set onmessage return adjust
        let tx_clone = tx.clone();
        data_channel.on_message(Box::new(
            move |msg: webrtc::data_channel::data_channel_message::DataChannelMessage| {
                // zero-copy： directly using msg.data (Bytes)
                let data = msg.data;
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
