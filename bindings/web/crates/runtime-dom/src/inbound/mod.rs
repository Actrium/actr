//! Inbound message handling for DOM
//!
//! DOM 侧消息接收处理：
//! 1. 从 SW 转发来的 Fast Path 消息
//! 2. WebRTC DataChannel 直接接收的消息

mod dispatcher;
mod webrtc_receiver;

pub use dispatcher::DomInboundDispatcher;
pub use webrtc_receiver::WebRtcDataChannelReceiver;
