use super::*;
use crate::lifecycle::credential_manager::{CredentialManager, RegistrationContext};
use crate::lifecycle::session_state::{SessionSnapshot, SessionState};
use actr_protocol::prost::Message as _;
use actr_protocol::{
    AIdCredential, ActrId, ActrType, IdentityClaims, Realm, RegisterRequest, RegisterResponse,
    RenewCredentialResponse, TurnCredential, register_response, renew_credential_response,
};
use prost::bytes::Bytes;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

fn actor(serial: u64) -> ActrId {
    ActrId {
        realm: Realm { realm_id: 7 },
        serial_number: serial,
        r#type: ActrType {
            manufacturer: "acme".to_string(),
            name: "node".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

fn credential_for_actor(actor_id: &ActrId, key_id: u32, expires_at: u64) -> AIdCredential {
    AIdCredential {
        key_id,
        claims: IdentityClaims {
            realm_id: actor_id.realm.realm_id,
            actor_id: actor_id.to_string_repr(),
            expires_at,
        }
        .encode_to_vec()
        .into(),
        signature: Bytes::from(vec![0; 64]),
    }
}

fn register_ok(actor_id: &ActrId, key_id: u32) -> register_response::RegisterOk {
    register_response::RegisterOk {
        actr_id: actor_id.clone(),
        credential: credential_for_actor(actor_id, key_id, 4_000_000_000),
        turn_credential: TurnCredential {
            username: "4000000000:actor".to_string(),
            password: "turn-password".to_string(),
            expires_at: 4_000_000_000,
        },
        credential_expires_at: Some(prost_types::Timestamp {
            seconds: 4_000_000_000,
            nanos: 0,
        }),
        signaling_heartbeat_interval_secs: 30,
        signing_pubkey: Bytes::from(vec![1; 32]),
        signing_key_id: key_id,
        renewal_token: Some(Bytes::from(vec![8; 32])),
        renewal_token_expires_at: Some(prost_types::Timestamp {
            seconds: 5_000_000_000,
            nanos: 0,
        }),
    }
}

fn session(actor_id: &ActrId, revision: u64, token_usable: bool) -> SessionState {
    SessionState::new(SessionSnapshot {
        actor_id: actor_id.clone(),
        credential: credential_for_actor(actor_id, 1, 4_000_000_000),
        credential_expires_at: prost_types::Timestamp {
            seconds: 4_000_000_000,
            nanos: 0,
        },
        turn_credential: TurnCredential {
            username: "old".to_string(),
            password: "old".to_string(),
            expires_at: 4_000_000_000,
        },
        renewal_token: if token_usable {
            Bytes::from(vec![7; 32])
        } else {
            Bytes::new()
        },
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: if token_usable { 5_000_000_000 } else { 1 },
            nanos: 0,
        },
        generation: revision,
    })
}

fn linked_ctx(actor_id: &ActrId) -> RegistrationContext {
    RegistrationContext::Linked {
        request: RegisterRequest {
            actr_type: actor_id.r#type.clone(),
            realm: actor_id.realm,
            ..Default::default()
        },
        realm_secret: None,
    }
}

fn seed(actor_id: &ActrId, revision: u64) -> PublishedCredential {
    PublishedCredential {
        credential: credential_for_actor(actor_id, 1, 4_000_000_000),
        credential_expires_at: None,
        turn_credential: None,
        actor_id: actor_id.clone(),
        revision,
    }
}

fn controller(
    actor_id: &ActrId,
    session: SessionState,
    endpoint: String,
    wake: Arc<dyn Fn() + Send + Sync>,
    shutdown: CancellationToken,
) -> (MembershipController, MembershipHandle) {
    let revision = session.generation_sync().unwrap();
    MembershipController::new(
        CredentialManager::new(session, linked_ctx(actor_id), endpoint, None),
        seed(actor_id, revision),
        wake,
        shutdown,
    )
}

#[tokio::test]
async fn soft_renew_advances_publication_revision_without_bumping_session_generation() {
    let actor_id = actor(1);
    let mut server = mockito::Server::new_async().await;
    let response = RenewCredentialResponse {
        result: Some(renew_credential_response::Result::Success(register_ok(
            &actor_id, 9,
        ))),
    };
    let mock = server
        .mock("POST", "/renew")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    let session = session(&actor_id, 1, true);
    let shutdown = CancellationToken::new();
    let (controller, handle) = controller(
        &actor_id,
        session.clone(),
        server.url(),
        Arc::new(|| {}),
        shutdown.clone(),
    );
    let task = spawn_membership_controller(controller);

    assert_eq!(
        handle.resolve(AuthVerdict::Rejected, 1).await,
        MembershipResolution::Published
    );
    assert_eq!(handle.current_revision(), 2);
    assert_eq!(session.generation().await, 1);
    assert_eq!(handle.credential_rx().borrow().credential.key_id, 9);

    shutdown.cancel();
    task.await.unwrap();
    mock.assert_async().await;
}

#[tokio::test]
async fn concurrent_same_revision_reports_do_one_reissue() {
    let actor_id = actor(1);
    let mut server = mockito::Server::new_async().await;
    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok(
            &actor_id, 9,
        ))),
    };
    let mock = server
        .mock("POST", "/reissue")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    let shutdown = CancellationToken::new();
    let (controller, handle) = controller(
        &actor_id,
        session(&actor_id, 1, false),
        server.url(),
        Arc::new(|| {}),
        shutdown.clone(),
    );
    let task = spawn_membership_controller(controller);

    let requests = (0..8).map(|_| {
        let handle = handle.clone();
        tokio::spawn(async move { handle.resolve(AuthVerdict::Rejected, 1).await })
    });
    let mut published = 0;
    for request in requests {
        match request.await.unwrap() {
            MembershipResolution::Published => published += 1,
            MembershipResolution::Superseded => {}
            other => panic!("unexpected resolution: {other:?}"),
        }
    }

    assert_eq!(published, 1);
    assert_eq!(handle.current_revision(), 2);
    shutdown.cancel();
    task.await.unwrap();
    mock.assert_async().await;
}

#[tokio::test]
async fn transient_failures_resolve_and_later_external_trigger_recovers() {
    let actor_id = actor(1);
    let mut server = mockito::Server::new_async().await;
    let unavailable = server
        .mock("POST", "/reissue")
        .with_status(503)
        .expect(2)
        .create_async()
        .await;

    let shutdown = CancellationToken::new();
    let (controller, handle) = controller(
        &actor_id,
        session(&actor_id, 1, false),
        server.url(),
        Arc::new(|| {}),
        shutdown.clone(),
    );
    let task = spawn_membership_controller(controller);

    assert_eq!(
        handle.resolve(AuthVerdict::Rejected, 1).await,
        MembershipResolution::Deferred
    );
    assert_eq!(handle.current_revision(), 1);
    assert_eq!(
        handle.resolve(AuthVerdict::Rejected, 1).await,
        MembershipResolution::Deferred
    );
    assert_eq!(handle.current_revision(), 1);
    unavailable.assert_async().await;
    unavailable.remove_async().await;

    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok(
            &actor_id, 9,
        ))),
    };
    let recovered = server
        .mock("POST", "/reissue")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;
    assert_eq!(
        handle.resolve(AuthVerdict::Rejected, 1).await,
        MembershipResolution::Published
    );
    assert_eq!(handle.current_revision(), 2);

    shutdown.cancel();
    task.await.unwrap();
    recovered.assert_async().await;
}

#[tokio::test]
async fn controller_has_no_background_refresh_or_denied_reprobe() {
    let actor_id = actor(1);
    let mut server = mockito::Server::new_async().await;
    let renew = server.mock("POST", "/renew").expect(0).create_async().await;
    let reissue = server
        .mock("POST", "/reissue")
        .expect(0)
        .create_async()
        .await;
    let shutdown = CancellationToken::new();
    let (controller, handle) = controller(
        &actor_id,
        session(&actor_id, 1, true),
        server.url(),
        Arc::new(|| {}),
        shutdown.clone(),
    );
    let task = spawn_membership_controller(controller);

    assert_eq!(
        handle.resolve(AuthVerdict::RealmDenied, 1).await,
        MembershipResolution::Denied
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    shutdown.cancel();
    task.await.unwrap();
    renew.assert_async().await;
    reissue.assert_async().await;
}

#[tokio::test]
async fn publish_is_visible_before_socket_wake() {
    let actor_id = actor(1);
    let mut server = mockito::Server::new_async().await;
    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok(
            &actor_id, 9,
        ))),
    };
    let _mock = server
        .mock("POST", "/reissue")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    let observed = Arc::new(AtomicU64::new(0));
    let receiver_slot = Arc::new(std::sync::Mutex::new(
        None::<watch::Receiver<Arc<PublishedCredential>>>,
    ));
    let observed_for_wake = observed.clone();
    let receiver_for_wake = receiver_slot.clone();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if let Some(receiver) = receiver_for_wake.lock().unwrap().as_ref() {
            observed_for_wake.store(receiver.borrow().revision, Ordering::SeqCst);
        }
    });

    let shutdown = CancellationToken::new();
    let (controller, handle) = controller(
        &actor_id,
        session(&actor_id, 1, false),
        server.url(),
        wake,
        shutdown.clone(),
    );
    *receiver_slot.lock().unwrap() = Some(handle.credential_rx());
    let task = spawn_membership_controller(controller);

    assert_eq!(
        handle.resolve(AuthVerdict::Rejected, 1).await,
        MembershipResolution::Published
    );
    assert_eq!(observed.load(Ordering::SeqCst), 2);

    shutdown.cancel();
    task.await.unwrap();
}
