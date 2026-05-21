//! Inbound message handling for DOM
//!
//! DOM-side inbound message handling:
//! 1. Fast Path messages forwarded from SW
//! 2. Messages received directly via WebRTC DataChannel

mod dispatcher;
mod webrtc_receiver;

pub use dispatcher::DomInboundDispatcher;
pub use webrtc_receiver::WebRtcDataChannelReceiver;
