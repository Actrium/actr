//! WebRTC subsystem
//!
//! Complete WebRTC P2P ConnectManage， package include ：
//! - signaling protocol quotient （Offer/Answer/ICE）
//! - Connect build independent andManage
//! - OutboundGate Implementation

pub(crate) mod connection; // WebRtcConnection Implementation
mod coordinator;
pub(crate) mod gate;
pub(crate) mod negotiator;
mod signaling;
pub(crate) mod trace;

// Re-export public WebRTC surface from this module boundary; internal hook
// plumbing stays crate-private.
pub use coordinator::WebRtcCoordinator;
pub use negotiator::WebRtcConfig;
pub use signaling::{
    ConnectionState, DisconnectReason, ReconnectConfig, SignalingClient, SignalingConfig,
    SignalingEvent, SignalingStats, WebSocketSignalingClient,
};
pub(crate) use signaling::{HookCallback, HookEvent};
