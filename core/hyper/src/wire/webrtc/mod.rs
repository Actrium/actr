//! WebRTC subsystem
//!
//! Complete WebRTC P2P ConnectManage， package include ：
//! - signaling protocol quotient （Offer/Answer/ICE）
//! - Connect build independent andManage
//! - OutboundGate Implementation

pub(crate) mod connection; // WebRtcConnection Implementation
pub mod coordinator;
pub(crate) mod gate;
pub(crate) mod negotiator;
pub mod signaling;
pub(crate) mod trace;

// Re-export core center Type. Submodule-internal structs (WebRtcConnection /
// WebRtcGate / WebRtcNegotiator) stay reachable via `webrtc::<module>::Name`
// for internal callers.
pub use coordinator::WebRtcCoordinator;
pub use negotiator::WebRtcConfig;
pub use signaling::{
    ConnectionState, DisconnectReason, ReconnectConfig, SignalingClient, SignalingConfig,
    SignalingEvent, SignalingStats, WebSocketSignalingClient,
};
