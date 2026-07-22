use super::*;
use crate::inbound::MediaFrameRegistry;
use crate::lifecycle::credential_manager::RegistrationContext;
use crate::lifecycle::membership::{MembershipController, PublishedCredential};
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
async fn membership_heartbeat_never_drives_disconnected_socket() {
    let actor_id = test_actor_id(1);
    let credential = test_credential_for_actor(&actor_id, 7, 4_000_000_000);
    let session = SessionState::new(SessionSnapshot {
        actor_id: actor_id.clone(),
        credential: credential.clone(),
        credential_expires_at: prost_types::Timestamp {
            seconds: 4_000_000_000,
            nanos: 0,
        },
        turn_credential: TurnCredential::default(),
        renewal_token: vec![7; 32].into(),
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: 5_000_000_000,
            nanos: 0,
        },
        generation: 1,
    });
    let manager = CredentialManager::new(
        session,
        RegistrationContext::Linked {
            request: RegisterRequest {
                actr_type: actor_id.r#type.clone(),
                realm: actor_id.realm,
                ..Default::default()
            },
            realm_secret: None,
        },
        "http://127.0.0.1:1",
        None,
    );
    let shutdown = CancellationToken::new();
    let (_controller, membership) = MembershipController::new(
        manager,
        PublishedCredential {
            credential: credential.clone(),
            credential_expires_at: None,
            turn_credential: None,
            actor_id: actor_id.clone(),
            revision: 1,
        },
        Arc::new(|| {}),
        shutdown.clone(),
    );
    let client = Arc::new(ReconnectBeforeHeartbeatClient::new_disconnected());
    let task = tokio::spawn(heartbeat_task(
        shutdown.clone(),
        client.clone() as Arc<dyn SignalingClient>,
        actor_id.clone(),
        CredentialState::new(credential, None, None),
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
        Some(membership),
    ));

    tokio::time::sleep(Duration::from_millis(75)).await;
    shutdown.cancel();
    task.await.unwrap();

    assert_eq!(client.connect_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.heartbeat_calls.load(Ordering::SeqCst), 0);
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
