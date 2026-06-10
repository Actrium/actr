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
// plumbing stays crate-private except under test-utils, where integration
// tests can install recorders without standing up a full node.
#[cfg(feature = "test-utils")]
pub use coordinator::WebRtcCoordinator;
#[cfg(not(feature = "test-utils"))]
pub(crate) use coordinator::WebRtcCoordinator;
pub(crate) use coordinator::{CleanupGuard, NetworkRecoveryStatus};
pub use negotiator::WebRtcConfig;
#[cfg(not(feature = "test-utils"))]
pub(crate) use signaling::WebSocketSignalingClient;
pub use signaling::{
    AuthConfig, AuthType, ConnectionState, DisconnectReason, ReconnectConfig, SignalingClient,
    SignalingConfig, SignalingEvent, SignalingStats,
};
#[cfg(not(feature = "test-utils"))]
pub(crate) use signaling::{HookCallback, HookEvent};
#[cfg(feature = "test-utils")]
pub use signaling::{HookCallback, HookEvent, WebSocketSignalingClient};
