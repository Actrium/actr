use super::*;
use crate::inbound::MediaFrameRegistry;
use crate::lifecycle::credential_manager::RegistrationContext;
use crate::lifecycle::{SessionSnapshot, SessionState};
use crate::transport::{NetworkError, NetworkResult};
use crate::wire::webrtc::{DisconnectReason, SignalingEvent, SignalingStats, WebRtcConfig};
use actr_protocol::prost::Message as _;
use actr_protocol::{
    AIdCredential, ActrType, CredentialWarning, IdentityClaims, Pong, Realm, RegisterResponse,
    RenewCredentialResponse, RouteCandidatesRequest, RouteCandidatesResponse, SignalingEnvelope,
    TurnCredential, UnregisterResponse, credential_warning, register_response,
    renew_credential_response,
};
use actr_runtime_mailbox::{MailboxStats, MessagePriority, MessageRecord};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, broadcast};
use uuid::Uuid;

fn test_actor_id(serial_number: u64) -> ActrId {
    ActrId {
        realm: Realm { realm_id: 1 },
        serial_number,
        r#type: ActrType {
            manufacturer: "acme".to_string(),
            name: "node".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

fn test_credential() -> AIdCredential {
    AIdCredential {
        key_id: 7,
        claims: bytes::Bytes::from_static(b"claims"),
        signature: bytes::Bytes::from(vec![0u8; 64]),
    }
}

fn test_credential_for_actor(actor_id: &ActrId, key_id: u32, expires_at: u64) -> AIdCredential {
    let claims = IdentityClaims {
        realm_id: actor_id.realm.realm_id,
        actor_id: actor_id.to_string_repr(),
        expires_at,
    };
    AIdCredential {
        key_id,
        claims: claims.encode_to_vec().into(),
        signature: bytes::Bytes::from(vec![0u8; 64]),
    }
}

struct EmptyMailbox;

#[async_trait::async_trait]
impl Mailbox for EmptyMailbox {
    async fn enqueue(
        &self,
        _from: Vec<u8>,
        _payload: Vec<u8>,
        _priority: MessagePriority,
    ) -> actr_runtime_mailbox::StorageResult<Uuid> {
        unimplemented!("not used by this test")
    }

    async fn dequeue(&self) -> actr_runtime_mailbox::StorageResult<Vec<MessageRecord>> {
        unimplemented!("not used by this test")
    }

    async fn ack(&self, _message_id: Uuid) -> actr_runtime_mailbox::StorageResult<()> {
        unimplemented!("not used by this test")
    }

    async fn status(&self) -> actr_runtime_mailbox::StorageResult<MailboxStats> {
        Ok(MailboxStats {
            queued_messages: 0,
            inflight_messages: 0,
            queued_by_priority: HashMap::new(),
        })
    }
}

struct ExpiredHeartbeatSignalingClient {
    event_tx: broadcast::Sender<SignalingEvent>,
}

impl ExpiredHeartbeatSignalingClient {
    fn new() -> Self {
        let (event_tx, _rx) = broadcast::channel(16);
        Self { event_tx }
    }
}

#[async_trait::async_trait]
impl SignalingClient for ExpiredHeartbeatSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        let _ = self.event_tx.send(SignalingEvent::Disconnected {
            reason: DisconnectReason::Manual,
        });
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Err(NetworkError::CredentialExpired(
            "Credential validation failed: Invalid credential format".to_string(),
        ))
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        unimplemented!("not used by this test")
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        unimplemented!("not used by this test")
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Ok(())
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn get_stats(&self) -> SignalingStats {
        SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

struct ReconnectBeforeHeartbeatClient {
    connected: AtomicBool,
    connect_calls: AtomicUsize,
    heartbeat_calls: AtomicUsize,
    heartbeat_sent: Notify,
}

struct WarningHeartbeatSignalingClient;

#[async_trait::async_trait]
impl SignalingClient for WarningHeartbeatSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Ok(Pong {
            seq: 1,
            suggest_interval_secs: None,
            credential_warning: Some(CredentialWarning {
                r#type: credential_warning::WarningType::KeyInTolerancePeriod as i32,
                message: "credential is expiring".to_string(),
            }),
        })
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        unimplemented!("not used by this test")
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        unimplemented!("not used by this test")
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Ok(())
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn get_stats(&self) -> SignalingStats {
        SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

impl ReconnectBeforeHeartbeatClient {
    fn new_disconnected() -> Self {
        Self {
            connected: AtomicBool::new(false),
            connect_calls: AtomicUsize::new(0),
            heartbeat_calls: AtomicUsize::new(0),
            heartbeat_sent: Notify::new(),
        }
    }

    fn new_connected() -> Self {
        let client = Self::new_disconnected();
        client.connected.store(true, Ordering::SeqCst);
        client
    }
}

#[async_trait::async_trait]
impl SignalingClient for ReconnectBeforeHeartbeatClient {
    async fn connect(&self) -> NetworkResult<()> {
        self.connect_calls.fetch_add(1, Ordering::SeqCst);
        self.connected.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        self.connect().await
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(NetworkError::ConnectionError(
                "Cannot send: WebSocket not connected".to_string(),
            ));
        }
        self.heartbeat_calls.fetch_add(1, Ordering::SeqCst);
        self.heartbeat_sent.notify_waiters();
        Ok(Pong {
            seq: 0,
            suggest_interval_secs: None,
            credential_warning: None,
        })
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        unimplemented!("not used by this test")
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        unimplemented!("not used by this test")
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Ok(())
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn get_stats(&self) -> SignalingStats {
        SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

#[tokio::test]
async fn heartbeat_tick_reconnects_before_sending_when_signaling_is_disconnected() {
    let actor_id = test_actor_id(1);
    let credential_state = CredentialState::new(test_credential(), None, None);
    let client = Arc::new(ReconnectBeforeHeartbeatClient::new_disconnected());
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(heartbeat_task(
        shutdown.clone(),
        client.clone() as Arc<dyn SignalingClient>,
        actor_id.clone(),
        credential_state,
        Arc::new(EmptyMailbox) as Arc<dyn Mailbox>,
        Duration::from_millis(20),
        RegisterRequest {
            actr_type: actor_id.r#type.clone(),
            realm: actor_id.realm,
            ..Default::default()
        },
        "http://127.0.0.1:1".to_string(),
        None,
        None,
        None,
        None,
        None,
    ));

    tokio::time::timeout(Duration::from_secs(5), async {
        while client.heartbeat_calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("heartbeat should be sent after reconnect preflight");

    shutdown.cancel();
    task.await.expect("heartbeat task should stop cleanly");

    assert_eq!(client.connect_calls.load(Ordering::SeqCst), 1);
    assert!(client.heartbeat_calls.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn credential_expiry_does_not_re_register_or_update_webrtc_local_id() {
    let initial_id = test_actor_id(1);
    let renewed_id = test_actor_id(2);
    let credential_state = CredentialState::new(test_credential(), None, None);
    let signaling_client = Arc::new(ExpiredHeartbeatSignalingClient::new());
    let coordinator = Arc::new(WebRtcCoordinator::new(
        initial_id.clone(),
        credential_state.clone(),
        signaling_client.clone(),
        WebRtcConfig::default(),
        Arc::new(MediaFrameRegistry::new()),
    ));

    let register_response = RegisterResponse {
        result: Some(register_response::Result::Success(
            register_response::RegisterOk {
                actr_id: renewed_id.clone(),
                credential: test_credential(),
                turn_credential: TurnCredential {
                    username: "1000:actor".to_string(),
                    password: "password".to_string(),
                    expires_at: 1000,
                },
                credential_expires_at: Some(prost_types::Timestamp {
                    seconds: 1000,
                    nanos: 0,
                }),
                signaling_heartbeat_interval_secs: 30,
                signing_pubkey: vec![0u8; 32].into(),
                signing_key_id: 7,
                renewal_token: None,
                renewal_token_expires_at: None,
            },
        )),
    };

    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(register_response.encode_to_vec())
        .create_async()
        .await;

    let mut consecutive_failures = 0;
    let updated_id = send_heartbeat_and_handle_response(
        &(signaling_client as Arc<dyn SignalingClient>),
        &initial_id,
        &credential_state,
        &(Arc::new(EmptyMailbox) as Arc<dyn Mailbox>),
        Duration::from_secs(30),
        &RegisterRequest {
            actr_type: renewed_id.r#type.clone(),
            realm: renewed_id.realm,
            ..Default::default()
        },
        &mut consecutive_failures,
        &server.url(),
        None,
        None,
        None,
        Some(&coordinator),
        None,
    )
    .await;

    assert_eq!(updated_id, None);
    assert_eq!(coordinator.local_id_for_test(), initial_id);
}

#[tokio::test]
async fn credential_warning_triggers_credential_manager_renewal() {
    const OLD_EXPIRY: i64 = 4_000_000_000;
    const NEW_EXPIRY: i64 = 4_000_001_000;
    let actor_id = test_actor_id(1);
    let mut server = mockito::Server::new_async().await;
    let renew_response = RenewCredentialResponse {
        result: Some(renew_credential_response::Result::Success(
            register_response::RegisterOk {
                actr_id: actor_id.clone(),
                credential: test_credential_for_actor(&actor_id, 9, NEW_EXPIRY as u64),
                turn_credential: TurnCredential {
                    username: "4000001000:actor".to_string(),
                    password: "new-password".to_string(),
                    expires_at: NEW_EXPIRY as u64,
                },
                credential_expires_at: Some(prost_types::Timestamp {
                    seconds: NEW_EXPIRY,
                    nanos: 0,
                }),
                signaling_heartbeat_interval_secs: 30,
                signing_pubkey: vec![0u8; 32].into(),
                signing_key_id: 9,
                renewal_token: Some(vec![8; 32].into()),
                renewal_token_expires_at: Some(prost_types::Timestamp {
                    seconds: NEW_EXPIRY + 1000,
                    nanos: 0,
                }),
            },
        )),
    };
    let mock = server
        .mock("POST", "/renew")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(renew_response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    let session = SessionState::new(SessionSnapshot {
        actor_id: actor_id.clone(),
        credential: test_credential_for_actor(&actor_id, 7, OLD_EXPIRY as u64),
        credential_expires_at: prost_types::Timestamp {
            seconds: OLD_EXPIRY,
            nanos: 0,
        },
        turn_credential: TurnCredential {
            username: "4000000000:actor".to_string(),
            password: "old-password".to_string(),
            expires_at: OLD_EXPIRY as u64,
        },
        renewal_token: vec![7; 32].into(),
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: OLD_EXPIRY + 1000,
            nanos: 0,
        },
        generation: 1,
    });
    let manager = CredentialManager::new(
        session.clone(),
        RegistrationContext::Linked {
            request: RegisterRequest {
                actr_type: actor_id.r#type.clone(),
                realm: actor_id.realm,
                ..Default::default()
            },
            realm_secret: None,
        },
        server.url(),
        None,
    );

    let mut consecutive_failures = 0;
    let updated_id = send_heartbeat_and_handle_response(
        &(Arc::new(WarningHeartbeatSignalingClient) as Arc<dyn SignalingClient>),
        &actor_id,
        &CredentialState::new(
            test_credential_for_actor(&actor_id, 7, OLD_EXPIRY as u64),
            None,
            None,
        ),
        &(Arc::new(EmptyMailbox) as Arc<dyn Mailbox>),
        Duration::from_secs(30),
        &RegisterRequest {
            actr_type: actor_id.r#type.clone(),
            realm: actor_id.realm,
            ..Default::default()
        },
        &mut consecutive_failures,
        &server.url(),
        None,
        Some(&manager),
        None,
        None,
        None,
    )
    .await;

    assert_eq!(updated_id, None);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if session.credential().await.key_id == 9 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("credential warning should trigger soft renewal");
    mock.assert_async().await;
}

use actr_protocol::RegisterAuthMode;
use std::sync::Mutex;

fn test_realm_secret_actor_id(serial_number: u64) -> ActrId {
    ActrId {
        realm: Realm { realm_id: 7 },
        serial_number,
        r#type: ActrType {
            manufacturer: "demo2".to_string(),
            name: "DuplexStreamService".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

fn test_realm_secret_credential(key_id: u32) -> AIdCredential {
    AIdCredential {
        key_id,
        claims: bytes::Bytes::from_static(b"claims"),
        signature: bytes::Bytes::from_static(&[7; 64]),
    }
}

#[derive(Default)]
struct FakeSignalingClient {
    actor_id: Mutex<Option<ActrId>>,
}

#[async_trait::async_trait]
impl SignalingClient for FakeSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Ok(())
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn get_stats(&self) -> SignalingStats {
        SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }

    async fn set_actor_id(&self, actor_id: ActrId) {
        *self.actor_id.lock().expect("actor_id mutex poisoned") = Some(actor_id);
    }

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

#[tokio::test]
async fn re_registration_sends_realm_secret_to_ais() {
    let mut server = mockito::Server::new_async().await;
    let new_actor_id = test_realm_secret_actor_id(42);
    let register_response = RegisterResponse {
        result: Some(register_response::Result::Success(
            register_response::RegisterOk {
                actr_id: new_actor_id.clone(),
                credential: test_realm_secret_credential(2),
                turn_credential: TurnCredential {
                    username: "turn-user".to_string(),
                    password: "turn-password".to_string(),
                    expires_at: 123,
                },
                credential_expires_at: Some(prost_types::Timestamp {
                    seconds: 456,
                    nanos: 0,
                }),
                signaling_heartbeat_interval_secs: 30,
                signing_pubkey: bytes::Bytes::from_static(&[1; 32]),
                signing_key_id: 2,
                renewal_token: None,
                renewal_token_expires_at: None,
            },
        )),
    };
    let _mock = server
        .mock("POST", "/register")
        .match_header("x-actrix-realm-secret", "rs_test_secret")
        .with_status(200)
        .with_body(register_response.encode_to_vec())
        .create_async()
        .await;

    let old_actor_id = test_realm_secret_actor_id(1);
    let credential_state = CredentialState::new(
        test_realm_secret_credential(1),
        Some(prost_types::Timestamp {
            seconds: 123,
            nanos: 0,
        }),
        None,
    );
    let register_request = RegisterRequest {
        actr_type: old_actor_id.r#type.clone(),
        realm: old_actor_id.realm,
        auth_mode: Some(RegisterAuthMode::Linked as i32),
        ..Default::default()
    };
    let client = Arc::new(FakeSignalingClient::default());

    let returned_actor_id = re_register_task(
        client,
        old_actor_id,
        register_request,
        credential_state,
        server.url(),
        Some("rs_test_secret".to_string()),
        None,
    )
    .await;

    assert_eq!(returned_actor_id, new_actor_id);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Virtual-time heartbeat pacing and ping-timeout boundaries (RFC-0419 §6)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Signaling client whose `send_heartbeat` never resolves. Records that the
/// send was entered so tests can anchor the `ping_timeout` deadline at the
/// exact virtual instant the timer was created.
struct StalledHeartbeatClient {
    heartbeat_entered: AtomicBool,
}

impl StalledHeartbeatClient {
    fn new() -> Self {
        Self {
            heartbeat_entered: AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl SignalingClient for StalledHeartbeatClient {
    async fn connect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        unimplemented!("not used by this test")
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        self.heartbeat_entered.store(true, Ordering::SeqCst);
        std::future::pending::<()>().await;
        unreachable!("stalled heartbeat never resolves")
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        unimplemented!("not used by this test")
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        unimplemented!("not used by this test")
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Ok(())
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn get_stats(&self) -> SignalingStats {
        SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

/// Poll the scheduler without moving virtual time. Asserting an unchanged
/// counter after this proves "no fire between advances" exactly: while the
/// test task only yields, the paused clock cannot auto-advance.
async fn yield_many(iterations: usize) {
    for _ in 0..iterations {
        tokio::task::yield_now().await;
    }
}

/// Wait until `condition` holds, mostly by yielding. The heartbeat path
/// contains one internal short timer (the bounded power-reserve fetch), so a
/// stalled condition is unstuck by small virtual advances. The total virtual
/// advance is bounded (≤ 10 s) so callers can budget it against their
/// interval slack; panics when the budget runs out.
async fn wait_for_with_bounded_advance(label: &str, mut condition: impl FnMut() -> bool) {
    let mut advanced = Duration::ZERO;
    for _ in 0..200 {
        for _ in 0..50 {
            if condition() {
                return;
            }
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(50)).await;
        advanced += Duration::from_millis(50);
    }
    panic!("timed out waiting for {label} (virtual advance budget exhausted at {advanced:?})");
}

fn heartbeat_register_request(actor_id: &ActrId) -> RegisterRequest {
    RegisterRequest {
        actr_type: actor_id.r#type.clone(),
        realm: actor_id.realm,
        ..Default::default()
    }
}

/// R2: under the paused (virtual) clock, heartbeats fire exactly on the
/// configured interval — never before the boundary, exactly one per boundary.
/// The generous interval/slack ratio absorbs the ≤ 10 s virtual creep the
/// bounded-advance waits may introduce around each observation.
#[tokio::test(start_paused = true)]
async fn heartbeat_ticks_track_the_configured_interval_in_virtual_time() {
    const INTERVAL: Duration = Duration::from_secs(3600);
    const SLACK: Duration = Duration::from_secs(60);

    let actor_id = test_actor_id(1);
    let client = Arc::new(ReconnectBeforeHeartbeatClient::new_connected());
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(heartbeat_task(
        shutdown.clone(),
        client.clone() as Arc<dyn SignalingClient>,
        actor_id.clone(),
        CredentialState::new(test_credential(), None, None),
        Arc::new(EmptyMailbox) as Arc<dyn Mailbox>,
        INTERVAL,
        heartbeat_register_request(&actor_id),
        "http://127.0.0.1:1".to_string(),
        None,
        None,
        None,
        None,
        None,
    ));

    // The first tick fires immediately on task start.
    wait_for_with_bounded_advance("first heartbeat", || {
        client.heartbeat_calls.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert_eq!(client.heartbeat_calls.load(Ordering::SeqCst), 1);

    for expected in 2usize..=3 {
        // Just short of the next boundary nothing may fire (no-early-fire).
        tokio::time::advance(INTERVAL - SLACK).await;
        yield_many(200).await;
        assert_eq!(
            client.heartbeat_calls.load(Ordering::SeqCst),
            expected - 1,
            "heartbeat must not fire before its interval elapses"
        );

        // Crossing the boundary releases exactly one more tick.
        tokio::time::advance(SLACK).await;
        wait_for_with_bounded_advance("next heartbeat", || {
            client.heartbeat_calls.load(Ordering::SeqCst) >= expected
        })
        .await;
        assert_eq!(client.heartbeat_calls.load(Ordering::SeqCst), expected);
    }

    shutdown.cancel();
    task.await.expect("heartbeat task should stop cleanly");
    assert_eq!(
        client.connect_calls.load(Ordering::SeqCst),
        0,
        "a connected client must not trigger the reconnect preflight"
    );
}

/// R2/R3: `tokio::time::interval` defaults to `MissedTickBehavior::Burst`, so
/// after a long pause of the heartbeat task (the virtual-time analogue of a
/// process suspension: the monotonic clock kept counting while the task never
/// ran) every missed boundary is delivered as a prompt catch-up tick — and
/// only the missed ones.
#[tokio::test(start_paused = true)]
async fn heartbeat_interval_bursts_missed_ticks_after_a_long_virtual_pause() {
    const INTERVAL: Duration = Duration::from_secs(60);

    let actor_id = test_actor_id(1);
    let client = Arc::new(ReconnectBeforeHeartbeatClient::new_connected());
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(heartbeat_task(
        shutdown.clone(),
        client.clone() as Arc<dyn SignalingClient>,
        actor_id.clone(),
        CredentialState::new(test_credential(), None, None),
        Arc::new(EmptyMailbox) as Arc<dyn Mailbox>,
        INTERVAL,
        heartbeat_register_request(&actor_id),
        "http://127.0.0.1:1".to_string(),
        None,
        None,
        None,
        None,
        None,
    ));

    wait_for_with_bounded_advance("first heartbeat", || {
        client.heartbeat_calls.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert_eq!(client.heartbeat_calls.load(Ordering::SeqCst), 1);

    // Jump over three missed boundaries in one advance.
    tokio::time::advance(INTERVAL * 3 + Duration::from_secs(30)).await;
    wait_for_with_bounded_advance("burst catch-up ticks", || {
        client.heartbeat_calls.load(Ordering::SeqCst) >= 4
    })
    .await;

    // Exactly the three missed ticks are delivered; the cadence then resumes
    // on the original schedule, so no further tick fires without an advance.
    yield_many(200).await;
    assert_eq!(
        client.heartbeat_calls.load(Ordering::SeqCst),
        4,
        "burst catch-up must deliver every missed tick and nothing more"
    );

    shutdown.cancel();
    task.await.expect("heartbeat task should stop cleanly");
}

/// Drive one stalled heartbeat send and pin the `ping_timeout` deadline with
/// a double-sided boundary: still pending one millisecond before the
/// deadline, resolved (as one consecutive failure) once the clock reaches it.
async fn assert_ping_timeout_boundary(heartbeat_interval: Duration, expected_timeout: Duration) {
    let actor_id = test_actor_id(1);
    let client = Arc::new(StalledHeartbeatClient::new());

    let task = tokio::spawn({
        let client = client.clone() as Arc<dyn SignalingClient>;
        let actor_id = actor_id.clone();
        async move {
            let mut consecutive_failures = 0u64;
            send_heartbeat_and_handle_response(
                &client,
                &actor_id,
                &CredentialState::new(test_credential(), None, None),
                &(Arc::new(EmptyMailbox) as Arc<dyn Mailbox>),
                heartbeat_interval,
                &heartbeat_register_request(&actor_id),
                &mut consecutive_failures,
                "http://127.0.0.1:1",
                None,
                None,
                None,
                None,
                None,
            )
            .await;
            consecutive_failures
        }
    });

    // Once the stalled send is entered, the timeout timer exists and its
    // deadline is anchored at the current virtual instant.
    wait_for_with_bounded_advance("stalled heartbeat send to start", || {
        client.heartbeat_entered.load(Ordering::SeqCst)
    })
    .await;

    tokio::time::advance(expected_timeout - Duration::from_millis(1)).await;
    yield_many(50).await;
    assert!(
        !task.is_finished(),
        "ping timeout must not fire before its {expected_timeout:?} deadline"
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    let consecutive_failures = task.await.expect("heartbeat send task should join");
    assert_eq!(
        consecutive_failures, 1,
        "a timed-out heartbeat must count exactly one consecutive failure"
    );
}

/// R2/R4: `ping_timeout = 0.4 × interval` when that product clears the floor.
/// A 30 s interval yields a 12 s timeout that must not fire at 11.999 s.
#[tokio::test(start_paused = true)]
async fn heartbeat_ping_timeout_fires_no_earlier_than_its_deadline() {
    assert_ping_timeout_boundary(Duration::from_secs(30), Duration::from_secs(12)).await;
}

/// R2/R4: the `max(0.4 × interval, 1 s)` floor. A 2 s interval would give
/// 0.8 s; the timeout must instead hold the full 1 s (pending at 999 ms).
#[tokio::test(start_paused = true)]
async fn heartbeat_ping_timeout_floors_at_one_second() {
    assert_ping_timeout_boundary(Duration::from_secs(2), Duration::from_secs(1)).await;
}
