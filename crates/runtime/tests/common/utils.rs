//! Common test utilities
//!
//! Helper functions for creating test actors, credentials, and peers

use actr_protocol::{AIdCredential, ActrId, ActrType, Realm};
use actr_runtime::inbound::MediaFrameRegistry;
use actr_runtime::lifecycle::CredentialState;
use actr_runtime::wire::webrtc::{
    SignalingClient, WebRtcConfig, WebRtcCoordinator, WebSocketSignalingClient,
};
use std::sync::Arc;

/// Create a test ActrId with the given serial number
pub fn make_actor_id(serial_number: u64) -> ActrId {
    ActrId {
        realm: Realm { realm_id: 1 },
        serial_number,
        r#type: ActrType {
            manufacturer: "acme".to_string(),
            name: "node".to_string(),
        },
    }
}

/// Create a dummy credential for testing
pub fn dummy_credential() -> AIdCredential {
    AIdCredential {
        encrypted_token: b"token".to_vec().into(),
        token_key_id: 7,
    }
}

/// Create a credential state for testing
pub fn create_credential_state_for_test(credential: AIdCredential) -> CredentialState {
    #[derive(Clone)]
    #[allow(dead_code)]
    struct CredentialStateInner {
        credential: AIdCredential,
        expires_at: Option<prost_types::Timestamp>,
        psk: Option<bytes::Bytes>,
    }

    let mock_psk = bytes::Bytes::from_static(b"mock_psk_for_testing_32_bytes!!");
    let inner = Arc::new(tokio::sync::RwLock::new(CredentialStateInner {
        credential,
        expires_at: None,
        psk: Some(mock_psk),
    }));

    unsafe { std::mem::transmute(inner) }
}

/// Create a WebRTC peer with WebSocket signaling
///
/// Returns both the coordinator and the signaling client
pub async fn create_peer_with_websocket(
    id: ActrId,
    server_url: &str,
) -> anyhow::Result<(Arc<WebRtcCoordinator>, Arc<dyn SignalingClient>)> {
    let credential = dummy_credential();
    let credential_state = create_credential_state_for_test(credential.clone());

    let signaling_client = WebSocketSignalingClient::connect_to(server_url)
        .await
        .expect("Failed to connect to test server");

    let config = WebRtcConfig::default();
    let media_registry = Arc::new(MediaFrameRegistry::new());

    let signaling_client_arc = signaling_client as Arc<dyn SignalingClient>;

    let coordinator = Arc::new(WebRtcCoordinator::new(
        id,
        credential_state,
        signaling_client_arc.clone(),
        config,
        1,
        media_registry,
    ));

    let c = coordinator.clone();
    tokio::spawn(async move {
        let _ = c.start().await;
    });

    Ok((coordinator, signaling_client_arc))
}
