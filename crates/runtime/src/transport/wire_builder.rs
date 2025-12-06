//! WireBuilder - Wire layer component builder
//!
//! Provides default Wire component builder implementation, supporting:
//! - WebRTC P2P connections (through WebRtcCoordinator)
//! - WebSocket C/S connections
//! - CancellationToken for terminating in-progress connection creation

use super::Dest; // Re-exported from actr-framework
use super::error::{NetworkError, NetworkResult};
use super::manager::WireBuilder;
use super::wire_handle::WireHandle;
use crate::wire::webrtc::WebRtcCoordinator;
use crate::wire::websocket::WebSocketConnection;
use async_trait::async_trait;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Default Wire builder configuration
pub struct DefaultWireBuilderConfig {
    /// WebSocket server URL template (can contain {actor_id} placeholder for dynamic substitution)
    pub websocket_url_template: Option<String>,

    /// Enable WebRTC
    pub enable_webrtc: bool,

    /// Enable WebSocket
    pub enable_websocket: bool,
}

impl Default for DefaultWireBuilderConfig {
    fn default() -> Self {
        Self {
            websocket_url_template: None,
            enable_webrtc: true,
            enable_websocket: false, // WebSocket disabled by default (requires URL configuration)
        }
    }
}

/// default Wire construct build device
///
/// based onconfigurationCreate WebRTC and/or WebSocket Wire group file 。
/// Supportsaturatedand format Connect（ same temporal attempt try multiple typeConnectType）。
pub struct DefaultWireBuilder {
    /// WebRTC coordinator（optional）
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,

    /// WebSocket URL vague template
    websocket_url_template: Option<String>,

    /// configuration
    config: DefaultWireBuilderConfig,
}

impl DefaultWireBuilder {
    /// Create new Wire construct build device
    ///
    /// # Arguments
    /// - `webrtc_coordinator`: WebRTC coordinator（If start usage WebRTC）
    /// - `config`: construct build device configuration
    pub fn new(
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        config: DefaultWireBuilderConfig,
    ) -> Self {
        Self {
            webrtc_coordinator,
            websocket_url_template: config.websocket_url_template.clone(),
            config,
        }
    }

    /// Build WebSocket URL from template
    fn build_websocket_url(&self, dest: &Dest) -> Option<String> {
        let template = self.websocket_url_template.as_ref()?;

        match dest {
            Dest::Actor(actor_id) => {
                // Replace {actor_id} placeholder with serial_number
                let url = template.replace("{actor_id}", &actor_id.serial_number.to_string());
                Some(url)
            }
            Dest::Shell | Dest::Local => {
                // Local/Shell calls don't need network connections (should be short-circuited at upper layer)
                // Return None for type completeness
                None
            }
        }
    }
}

#[async_trait]
impl WireBuilder for DefaultWireBuilder {
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn create_connections(&self, dest: &Dest) -> NetworkResult<Vec<WireHandle>> {
        // Delegate to method with no cancel token
        self.create_connections_with_cancel(dest, None).await
    }

    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn create_connections_with_cancel(
        &self,
        dest: &Dest,
        cancel_token: Option<CancellationToken>,
    ) -> NetworkResult<Vec<WireHandle>> {
        let mut connections = Vec::new();

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

        // 1. Check if already cancelled
        check_cancelled(&cancel_token)?;

        // 2. attempt try Create WebSocket Connect
        if self.config.enable_websocket {
            check_cancelled(&cancel_token)?;

            if let Some(url) = self.build_websocket_url(dest) {
                tracing::debug!("🏭 [Factory] Create WebSocket Connect: {}", url);
                let ws_conn = WebSocketConnection::new(url);
                connections.push(WireHandle::WebSocket(ws_conn));
            } else {
                tracing::warn!(
                    "⚠️ [Factory] WebSocket enabled but no method construct build URL: {:?}",
                    dest
                );
            }
        }

        // 3. Check cancellation before WebRTC
        check_cancelled(&cancel_token)?;

        // 4. attempt try Create WebRTC Connect
        if self.config.enable_webrtc {
            if let Some(coordinator) = &self.webrtc_coordinator {
                // WebRTC merely Support Actor Type
                if dest.is_actor() {
                    tracing::debug!("🏭 [Factory] Create WebRTC Connectto: {:?}", dest);

                    // Check cancellation before long-running operation
                    check_cancelled(&cancel_token)?;

                    match coordinator
                        .create_connection(dest, cancel_token.clone())
                        .await
                    {
                        Ok(webrtc_conn) => {
                            // Check cancellation after creation
                            if let Err(e) = check_cancelled(&cancel_token) {
                                // Clean up newly created connection
                                if let Err(close_err) = webrtc_conn.close().await {
                                    tracing::warn!(
                                        "⚠️ [Factory] Failed to close cancelled connection: {}",
                                        close_err
                                    );
                                }
                                return Err(e);
                            }
                            connections.push(WireHandle::WebRTC(webrtc_conn));
                        }
                        Err(e) => {
                            tracing::warn!(
                                "❌ [Factory] WebRTC ConnectCreatefailure: {:?}: {}",
                                dest,
                                e
                            );
                            // not ReturnsError，allowusingotherConnectType
                        }
                    }
                } else {
                    tracing::debug!(
                        "ℹ️ [Factory] WebRTC not Support Shell item mark ，skip through "
                    );
                }
            } else {
                tracing::warn!("⚠️ [Factory] WebRTC enabled but not Provide WebRtcCoordinator");
            }
        }

        tracing::info!(
            "✨ [Factory] as {:?} Create done {} Connect",
            dest,
            connections.len()
        );

        Ok(connections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::ActrId;

    #[test]
    fn test_build_websocket_url() {
        let config = DefaultWireBuilderConfig {
            websocket_url_template: Some("ws://server:8080/actor/{actor_id}".to_string()),
            enable_websocket: true,
            enable_webrtc: false,
        };

        let factory = DefaultWireBuilder::new(None, config);

        let mut actor_id = ActrId::default();
        actor_id.serial_number = 12345;
        let dest = Dest::Actor(actor_id);

        let url = factory.build_websocket_url(&dest);
        assert_eq!(url, Some("ws://server:8080/actor/12345".to_string()));
    }

    #[tokio::test]
    async fn test_create_websocket_connection() {
        use actr_protocol::ActrId;

        let config = DefaultWireBuilderConfig {
            websocket_url_template: Some("ws://localhost:8080".to_string()),
            enable_websocket: true,
            enable_webrtc: false,
        };

        let factory = DefaultWireBuilder::new(None, config);
        // Use Actor dest instead of Shell (Shell doesn't create network connections)
        let actor_id = ActrId::default();
        let dest = Dest::actor(actor_id);

        let connections = factory.create_connections(&dest).await.unwrap();
        assert_eq!(connections.len(), 1);

        if let WireHandle::WebSocket(_ws_conn) = &connections[0] {
            // WebSocket ConnectCreatesuccess
        } else {
            panic!("Expected WebSocket connection");
        }
    }
}
