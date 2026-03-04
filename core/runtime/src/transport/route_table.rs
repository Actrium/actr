//! PayloadType routing extension
//!
//! Provides static routing configuration for PayloadType

use actr_protocol::PayloadType;

/// DataChannel QoS configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataChannelQoS {
    /// Signaling: ordered, reliable
    Signal,

    /// Reliable: reliable transmission
    Reliable,

    /// Latency-first: allow packet loss
    LatencyFirst,
}

/// DataLane type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataLaneType {
    /// WebRTC DataChannel (with QoS)
    WebRtcDataChannel(DataChannelQoS),

    /// WebSocket
    WebSocket,
}

/// PayloadType routing extension
pub trait PayloadTypeExt {
    /// Get the list of supported DataLane types (ordered by priority)
    fn data_lane_types(self) -> &'static [DataLaneType];
}

impl PayloadTypeExt for PayloadType {
    #[inline]
    fn data_lane_types(self) -> &'static [DataLaneType] {
        match self {
            // RPC_RELIABLE - RpcEnvelope with reliable ordered transmission
            PayloadType::RpcReliable => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::Reliable),
                DataLaneType::WebSocket,
            ],

            // RPC_SIGNAL - RpcEnvelope with high-priority signaling channel
            PayloadType::RpcSignal => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::Signal),
                DataLaneType::WebSocket,
            ],

            // STREAM_RELIABLE - DataStream with reliable ordered transmission
            PayloadType::StreamReliable => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::Reliable),
                DataLaneType::WebSocket,
            ],

            // STREAM_LATENCY_FIRST - DataStream with low latency partial-reliable transmission
            PayloadType::StreamLatencyFirst => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::LatencyFirst),
                DataLaneType::WebSocket,
            ],

            // MEDIA_RTP - Not routed through DataLane, uses MediaFrameRegistry
            PayloadType::MediaRtp => &[],
        }
    }
}

impl DataLaneType {
    /// Determine if WebRTC connection is needed for this DataLane Type
    #[inline]
    pub fn needs_webrtc(self) -> bool {
        matches!(self, DataLaneType::WebRtcDataChannel(_))
    }

    /// Check if this DataLane Type supports WebSocket
    #[inline]
    pub fn supports_websocket(self) -> bool {
        matches!(self, DataLaneType::WebSocket)
    }
}
