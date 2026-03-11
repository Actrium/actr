//! Transport Layer - DOM Environment
//!
//! Transport implementations for the DOM side:
//! - WebRTC DataChannel lane for P2P data transport
//! - WebRTC MediaTrack lane for media transport
//! - PostMessage lane for communication with the Service Worker

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
