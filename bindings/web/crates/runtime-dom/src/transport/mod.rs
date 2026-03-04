//! Transport Layer - DOM Environment
//!
//! DOM 端的传输层实现：
//! - WebRTC DataChannel Lane：P2P 数据通道
//! - WebRTC MediaTrack Lane：媒体流通道
//! - PostMessage Lane：与 Service Worker 的通信通道

pub mod dom_transport;
pub mod lane;
pub mod postmessage;
pub mod webrtc_datachannel;
pub mod webrtc_mediatrack;

pub use dom_transport::DomTransport;
pub use lane::{DataLane, LaneResult};
pub use postmessage::PostMessageLaneBuilder;
pub use webrtc_datachannel::{WebRtcDataChannelLaneBuilder, create_datachannel_config};
pub use webrtc_mediatrack::{
    MediaTrackProcessor, MediaTrackType, WebRtcMediaTrackLaneBuilder,
    create_mediatrack_lane_with_processor,
};
