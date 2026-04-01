//! WireBuilder - Wire layer component builder
//!
//! Provides default Wire component builder implementation, supporting:
//! - WebRTC P2P connections (through WebRtcCoordinator)
//! - WebSocket C/S connections
//! - CancellationToken for terminating in-progress connection creation

use super::Dest; // Re-exported from actr-framework
use super::error::{NetworkError, NetworkResult};
use super::peer_transport::WireBuilder;
use super::wire_handle::WireHandle;
use crate::lifecycle::CredentialState;
use crate::wire::webrtc::WebRtcCoordinator;
use crate::wire::websocket::WebSocketConnection;
use actr_protocol::ActrId;
use actr_protocol::prost::Message as ProstMessage;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Default Wire builder configuration
pub struct DefaultWireBuilderConfig {
    /// Local node identity as hex-encoded protobuf `ActrId` bytes, sent in the `X-Actr-Source-ID` header during outbound WebSocket handshakes.
    pub local_id_hex: String,

    /// Enable WebRTC
    pub enable_webrtc: bool,

    /// Enable WebSocket
    pub enable_websocket: bool,

    /// Shared map of discovered WebSocket direct-connect URLs, keyed by ActrId.
    ///
    /// Populated by discovery flow after receiving ws_address info
    /// from the signaling server.  When a connection to an ActrId is needed and this map
    /// contains an entry for it, the stored URL is used instead of the url_template.
    pub discovered_ws_addresses: Arc<RwLock<HashMap<ActrId, String>>>,

    /// Optional local credential state. During outbound WebSocket handshakes the current credential is base64-encoded and sent in the `X-Actr-Credential` header so the peer can verify the Ed25519 signature.
    pub credential_state: Option<CredentialState>,
}

impl Default for DefaultWireBuilderConfig {
    fn default() -> Self {
        Self {
            local_id_hex: String::new(),
            enable_webrtc: true,
            enable_websocket: true,
            discovered_ws_addresses: Arc::new(RwLock::new(HashMap::new())),
            credential_state: None,
        }
    }
}

/// Default builder for wire-layer connections.
///
/// Creates WebRTC and/or WebSocket wire handles from configuration and supports attempting multiple connection types during the same creation pass.
pub struct DefaultWireBuilder {
    /// Optional WebRTC coordinator.
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,

    /// Local node identity hex string used as `X-Actr-Source-ID` in outbound WebSocket handshakes.
    local_id_hex: String,

    /// Shared map of discovered WebSocket URLs (from signaling discovery)
    discovered_ws_addresses: Arc<RwLock<HashMap<ActrId, String>>>,

    /// Local credential state used to provide `X-Actr-Credential` during outbound WebSocket handshakes.
    credential_state: Option<CredentialState>,

    /// Builder configuration.
    config: DefaultWireBuilderConfig,
}

impl DefaultWireBuilder {
    /// Create a new wire builder.
    ///
    /// # Arguments
    /// - `webrtc_coordinator`: WebRTC coordinator when WebRTC support is enabled
    /// - `config`: builder configuration
    pub fn new(
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        config: DefaultWireBuilderConfig,
    ) -> Self {
        Self {
            webrtc_coordinator,
            local_id_hex: config.local_id_hex.clone(),
            discovered_ws_addresses: config.discovered_ws_addresses.clone(),
            credential_state: config.credential_state.clone(),
            config,
        }
    }

    /// Look up the direct WebSocket URL for the target node, sourced only from service discovery.
    async fn resolve_websocket_url(&self, dest: &Dest) -> Option<String> {
        if let Dest::Actor(actor_id) = dest {
            let map = self.discovered_ws_addresses.read().await;
            if let Some(url) = map.get(actor_id) {
                tracing::debug!(
                    "🔎 [Factory] Using discovered WebSocket URL for {}: {}",
                    actor_id,
                    url
                );
                return Some(url.clone());
            }
        }
        None
    }
}

#[async_trait]
impl WireBuilder for DefaultWireBuilder {
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn create_connections(&self, dest: &Dest) -> NetworkResult<Vec<Arc<dyn WireHandle>>> {
        // Delegate to method with no cancel token
        self.create_connections_with_cancel(dest, None).await
    }

    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn create_connections_with_cancel(
        &self,
        dest: &Dest,
        cancel_token: Option<CancellationToken>,
    ) -> NetworkResult<Vec<Arc<dyn WireHandle>>> {
        let mut connections: Vec<Arc<dyn WireHandle>> = Vec::new();

        // Helper to check cancellation
        let check_cancelled = |token: &Option<CancellationToken>| -> NetworkResult<()> {
            if let Some(t) = token {
                if t.is_cancelled() {
                    return Err(NetworkError::ConnectionClosed(
                        "Connection creation cancelled".to_string(),
                    ));
                }
            }
            Ok(())
        };

        // 1. Check whether the operation was already cancelled.
        check_cancelled(&cancel_token)?;

        // 2. Try to establish a WebSocket connection.
        // The URL comes from service discovery (`discovered_ws_addresses`). If nothing was discovered, skip WebSocket for this attempt.
        if self.config.enable_websocket {
            check_cancelled(&cancel_token)?;

            if let Some(url) = self.resolve_websocket_url(dest).await {
                tracing::debug!("🏭 [Factory] Create WebSocket Connect: {}", url);
                let mut ws_conn =
                    WebSocketConnection::new(url).with_local_id(self.local_id_hex.clone());

                // Attach the local credential so the peer `WebSocketGate` can verify the Ed25519 signature.
                if let Some(ref cred_state) = self.credential_state {
                    let credential = cred_state.credential().await;
                    let cred_bytes = credential.encode_to_vec();
                    use base64::Engine as _;
                    let cred_b64 = base64::engine::general_purpose::STANDARD.encode(&cred_bytes);
                    ws_conn = ws_conn.with_credential_b64(cred_b64);
                }

                connections.push(Arc::new(ws_conn) as Arc<dyn WireHandle>);
            } else {
                tracing::debug!(
                    "🔎 [Factory] No WebSocket URL available for {:?}, skipping WS connection",
                    dest
                );
            }
        }

        // 3. Check cancellation before trying WebRTC.
        check_cancelled(&cancel_token)?;

        // 4. Attempt to create a WebRTC connection.
        if self.config.enable_webrtc {
            if let Some(coordinator) = &self.webrtc_coordinator {
                // WebRTC is only supported for actor destinations.
                if dest.is_actor() {
                    tracing::debug!("🏭 [Factory] Creating WebRTC connection to: {:?}", dest);

                    // Check cancellation before long-running operation
                    check_cancelled(&cancel_token)?;

                    match coordinator
                        .create_connection(dest, cancel_token.clone())
                        .await
                    {
                        Ok(webrtc_conn) => {
                            // Check cancellation again after creation.
                            if let Err(e) = check_cancelled(&cancel_token) {
                                // Clean up the newly created connection.
                                if let Err(close_err) = webrtc_conn.close().await {
                                    tracing::warn!(
                                        "⚠️ [Factory] Failed to close cancelled connection: {}",
                                        close_err
                                    );
                                }
                                return Err(e);
                            }
                            connections.push(Arc::new(webrtc_conn) as Arc<dyn WireHandle>);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "❌ [Factory] WebRTC connection creation failed: {:?}: {}",
                                dest,
                                e
                            );
                            // Do not return an error here; allow other connection types to proceed.
                        }
                    }
                } else {
                    tracing::debug!(
                        "ℹ️ [Factory] WebRTC does not support this destination type, skipping"
                    );
                }
            } else {
                tracing::warn!(
                    "⚠️ [Factory] WebRTC is enabled but no WebRtcCoordinator was provided"
                );
            }
        }

        tracing::info!(
            "✨ [Factory] Finished creating {} connections for {:?}",
            connections.len(),
            dest,
        );

        Ok(connections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ConnType;
    use actr_protocol::ActrId;

    #[tokio::test]
    async fn test_no_ws_connection_without_discovery() {
        // WebSocket URLs come only from service discovery; without a discovery record no WS connection should be created.
        let config = DefaultWireBuilderConfig {
            enable_websocket: true,
            enable_webrtc: false,
            local_id_hex: "deadbeef".to_string(),
            discovered_ws_addresses: Arc::new(RwLock::new(HashMap::new())),
            credential_state: None,
        };
        let factory = DefaultWireBuilder::new(None, config);
        let dest = Dest::actor(ActrId::default());
        let connections = factory.create_connections(&dest).await.unwrap();
        assert!(connections.is_empty());
    }

    #[tokio::test]
    async fn test_ws_connection_from_discovery() {
        // A discovered address should allow a WS connection to be created.
        let map = Arc::new(RwLock::new(HashMap::new()));
        let actor_id = ActrId::default();
        map.write()
            .await
            .insert(actor_id.clone(), "ws://localhost:9001".to_string());

        let config = DefaultWireBuilderConfig {
            enable_websocket: true,
            enable_webrtc: false,
            local_id_hex: "deadbeef".to_string(),
            discovered_ws_addresses: map,
            credential_state: None,
        };
        let factory = DefaultWireBuilder::new(None, config);
        let dest = Dest::actor(actor_id);
        let connections = factory.create_connections(&dest).await.unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].connection_type(), ConnType::WebSocket);
    }
}
