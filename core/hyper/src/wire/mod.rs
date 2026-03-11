//! Wire Layer 0: Physical wire layer
//!
//! Low-level transport implementations:
//! - webrtc: WebRTC transport (DataChannel, MediaTrack, Coordinator, Signaling)
//! - websocket: WebSocket transport
//!
//! **Note**: For intra-process communication, use `crate::transport::InprocTransportManager`

pub mod webrtc;
pub mod websocket;

use crate::key_cache::KeyFetcher;
use actr_protocol::{AIdCredential, ActrId};
use async_trait::async_trait;

/// SignalingClient 到 KeyFetcher 的适配器
///
/// `KeyFetcher::fetch_key` 只接受 `key_id`，而 `SignalingClient::get_signing_key` 还需要
/// `actor_id` 和 `credential`。此适配器持有上下文，将调用转发给底层 signaling 客户端。
pub struct SignalingKeyFetcher {
    pub client: std::sync::Arc<dyn webrtc::SignalingClient>,
    pub actor_id: ActrId,
    pub credential: AIdCredential,
}

#[async_trait]
impl KeyFetcher for SignalingKeyFetcher {
    async fn fetch_key(&self, key_id: u32) -> crate::error::HyperResult<(u32, Vec<u8>)> {
        self.client
            .get_signing_key(self.actor_id.clone(), self.credential.clone(), key_id)
            .await
            .map_err(|e| {
                tracing::warn!(key_id, error = ?e, "SignalingKeyFetcher: 通过 signaling 拉取 AIS 公钥失败");
                crate::error::HyperError::AisBootstrapFailed(format!(
                    "signaling get_signing_key 失败: {e:?}"
                ))
            })
    }
}

// Re-export commonly used types
pub use webrtc::{
    AuthConfig, AuthType, IceServer, ReconnectConfig, SignalingClient, SignalingConfig,
    SignalingEvent, SignalingStats, WebRtcConfig, WebRtcConnection, WebRtcCoordinator, WebRtcGate,
    WebRtcNegotiator, WebSocketSignalingClient,
};
pub use websocket::{WebSocketConnection, WebSocketGate, WebSocketServer, WsAuthContext};
