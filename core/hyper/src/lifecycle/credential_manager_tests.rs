use super::*;
use actr_protocol::{
    AIdCredential, ActrId, ActrType, IdentityClaims, Realm, RegisterRequest, RegisterResponse,
    RenewCredentialResponse, TurnCredential, register_response, renew_credential_response,
};
use prost::bytes::Bytes;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

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

fn credential(key_id: u32) -> AIdCredential {
    AIdCredential {
        key_id,
        claims: Bytes::new(),
        signature: Bytes::from(vec![0; 64]),
    }
}

fn credential_for_actor(actor_id: &ActrId, key_id: u32, expires_at: u64) -> AIdCredential {
    let claims = IdentityClaims {
        realm_id: actor_id.realm.realm_id,
        actor_id: actor_id.to_string_repr(),
        expires_at,
    };
    AIdCredential {
        key_id,
        claims: claims.encode_to_vec().into(),
        signature: Bytes::from(vec![0; 64]),
    }
}

fn register_ok(serial: u64, key_id: u32) -> register_response::RegisterOk {
    register_response::RegisterOk {
        actr_id: actor(serial),
        credential: credential(key_id),
        turn_credential: TurnCredential {
            username: format!("1000:actor-{serial}"),
            password: "turn-password".to_string(),
            expires_at: 1000,
        },
        credential_expires_at: Some(prost_types::Timestamp {
            seconds: 1000,
            nanos: 0,
        }),
        signaling_heartbeat_interval_secs: 30,
        signing_pubkey: Bytes::from(vec![1; 32]),
        signing_key_id: key_id,
        renewal_token: Some(Bytes::from_static(b"new-renewal-token-32-bytes!!")),
        renewal_token_expires_at: Some(prost_types::Timestamp {
            seconds: 2000,
            nanos: 0,
        }),
    }
}

#[tokio::test]
async fn expired_renewal_token_reissues_for_stable_actor_id() {
    let mut server = mockito::Server::new_async().await;
    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok(1, 9))),
    };
    let mock = server
        .mock("POST", "/reissue")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    let session = SessionState::new(super::super::session_state::SessionSnapshot {
        actor_id: actor(1),
        credential: credential(1),
        credential_expires_at: prost_types::Timestamp {
            seconds: 10,
            nanos: 0,
        },
        turn_credential: TurnCredential {
            username: "10:actor-1".to_string(),
            password: "old".to_string(),
            expires_at: 10,
        },
        renewal_token: Bytes::from_static(b"expired-renewal-token-32bytes"),
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: 1,
            nanos: 0,
        },
        generation: 1,
    });

    let request = RegisterRequest {
        actr_type: actor(1).r#type,
        realm: Realm { realm_id: 7 },
        ..Default::default()
    };

    run_renewal_once(
        session.clone(),
        server.url(),
        None,
        RegistrationContext::Linked {
            request,
            realm_secret: None,
        },
        None,
        None,
    )
    .await
    .expect("hard rebind should commit new snapshot");

    mock.assert_async().await;
    let snapshot = session.snapshot().await;
    assert_eq!(snapshot.actor_id, actor(1));
    assert_eq!(snapshot.credential.key_id, 9);
    assert_eq!(snapshot.generation, 2);
    assert_eq!(
        session.phase().await,
        super::super::session_state::SessionPhase::Active
    );
}

#[tokio::test]
async fn soft_renewal_fires_credential_renewed_hook() {
    const OLD_EXPIRY: i64 = 4_000_000_000;
    const NEW_EXPIRY: i64 = 4_000_001_000;
    let actor_id = actor(1);
    let mut server = mockito::Server::new_async().await;
    let response = RenewCredentialResponse {
        result: Some(renew_credential_response::Result::Success(
            register_response::RegisterOk {
                actr_id: actor_id.clone(),
                credential: credential_for_actor(&actor_id, 9, NEW_EXPIRY as u64),
                turn_credential: TurnCredential {
                    username: "4000001000:actor-1".to_string(),
                    password: "turn-password".to_string(),
                    expires_at: NEW_EXPIRY as u64,
                },
                credential_expires_at: Some(prost_types::Timestamp {
                    seconds: NEW_EXPIRY,
                    nanos: 0,
                }),
                signaling_heartbeat_interval_secs: 30,
                signing_pubkey: Bytes::from(vec![1; 32]),
                signing_key_id: 9,
                renewal_token: Some(Bytes::from(vec![8; 32])),
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
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    let session = SessionState::new(super::super::session_state::SessionSnapshot {
        actor_id: actor_id.clone(),
        credential: credential_for_actor(&actor_id, 1, OLD_EXPIRY as u64),
        credential_expires_at: prost_types::Timestamp {
            seconds: OLD_EXPIRY,
            nanos: 0,
        },
        turn_credential: TurnCredential {
            username: "4000000000:actor-1".to_string(),
            password: "old".to_string(),
            expires_at: OLD_EXPIRY as u64,
        },
        renewal_token: Bytes::from(vec![7; 32]),
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: OLD_EXPIRY + 1000,
            nanos: 0,
        },
        generation: 1,
    });

    let hook_expiry = Arc::new(AtomicU64::new(0));
    let hook_expiry_for_cb = hook_expiry.clone();
    let hook_callback: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_expiry = hook_expiry_for_cb.clone();
        Box::pin(async move {
            if let crate::wire::webrtc::HookEvent::CredentialRenewed { new_expiry } = event {
                let secs = new_expiry
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("expiry should be after epoch")
                    .as_secs();
                hook_expiry.store(secs, AtomicOrdering::SeqCst);
            }
        })
    });

    run_renewal_once(
        session,
        server.url(),
        None,
        RegistrationContext::Linked {
            request: RegisterRequest {
                actr_type: actor_id.r#type.clone(),
                realm: actor_id.realm,
                ..Default::default()
            },
            realm_secret: None,
        },
        None,
        Some(hook_callback),
    )
    .await
    .expect("soft renewal should succeed");

    mock.assert_async().await;
    assert_eq!(hook_expiry.load(AtomicOrdering::SeqCst), NEW_EXPIRY as u64);
}

/// P2 regression: the credential-only hard reissue (membership controller
/// path) commits new credential material and must fire the documented
/// CredentialRenewed notification exactly like the soft-renew path.
#[tokio::test]
async fn credential_only_hard_reissue_fires_credential_renewed_hook() {
    let mut server = mockito::Server::new_async().await;
    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok(1, 9))),
    };
    let mock = server
        .mock("POST", "/reissue")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    // Expired renewal token -> run_reacquire_once takes the hard-rebind branch.
    let session = SessionState::new(SessionSnapshot {
        actor_id: actor(1),
        credential: credential(1),
        credential_expires_at: prost_types::Timestamp {
            seconds: 10,
            nanos: 0,
        },
        turn_credential: TurnCredential {
            username: "10:actor-1".to_string(),
            password: "old".to_string(),
            expires_at: 10,
        },
        renewal_token: Bytes::from_static(b"expired-renewal-token-32bytes"),
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: 1,
            nanos: 0,
        },
        generation: 1,
    });

    let hook_expiry = Arc::new(AtomicU64::new(0));
    let hook_expiry_for_cb = hook_expiry.clone();
    let hook_callback: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_expiry = hook_expiry_for_cb.clone();
        Box::pin(async move {
            if let crate::wire::webrtc::HookEvent::CredentialRenewed { new_expiry } = event {
                let secs = new_expiry
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("expiry should be after epoch")
                    .as_secs();
                hook_expiry.store(secs, AtomicOrdering::SeqCst);
            }
        })
    });

    let outcome = run_reacquire_once(
        session,
        server.url(),
        None,
        RegistrationContext::Linked {
            request: RegisterRequest {
                actr_type: actor(1).r#type,
                realm: Realm { realm_id: 7 },
                ..Default::default()
            },
            realm_secret: None,
        },
        None,
        Some(hook_callback),
    )
    .await;

    mock.assert_async().await;
    assert!(
        matches!(outcome, ReacquireOutcome::Renewed(_)),
        "hard reissue should publish a renewed credential"
    );
    // register_ok(1, 9) advertises credential_expires_at = 1000s.
    assert_eq!(
        hook_expiry.load(AtomicOrdering::SeqCst),
        1000,
        "hard reissue must fire CredentialRenewed with the committed expiry"
    );
}

/// The legacy fire-and-forget hard rebind path has the same contract: after a
/// committed reissue, the workload observes CredentialRenewed.
#[tokio::test]
async fn legacy_hard_rebind_fires_credential_renewed_hook() {
    let mut server = mockito::Server::new_async().await;
    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok(1, 9))),
    };
    let mock = server
        .mock("POST", "/reissue")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    let session = SessionState::new(SessionSnapshot {
        actor_id: actor(1),
        credential: credential(1),
        credential_expires_at: prost_types::Timestamp {
            seconds: 10,
            nanos: 0,
        },
        turn_credential: TurnCredential {
            username: "10:actor-1".to_string(),
            password: "old".to_string(),
            expires_at: 10,
        },
        renewal_token: Bytes::from_static(b"expired-renewal-token-32bytes"),
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: 1,
            nanos: 0,
        },
        generation: 1,
    });

    let hook_expiry = Arc::new(AtomicU64::new(0));
    let hook_expiry_for_cb = hook_expiry.clone();
    let hook_callback: crate::wire::webrtc::HookCallback = Arc::new(move |event| {
        let hook_expiry = hook_expiry_for_cb.clone();
        Box::pin(async move {
            if let crate::wire::webrtc::HookEvent::CredentialRenewed { new_expiry } = event {
                let secs = new_expiry
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("expiry should be after epoch")
                    .as_secs();
                hook_expiry.store(secs, AtomicOrdering::SeqCst);
            }
        })
    });

    run_renewal_once(
        session,
        server.url(),
        None,
        RegistrationContext::Linked {
            request: RegisterRequest {
                actr_type: actor(1).r#type,
                realm: Realm { realm_id: 7 },
                ..Default::default()
            },
            realm_secret: None,
        },
        None,
        Some(hook_callback),
    )
    .await
    .expect("hard rebind should commit new snapshot");

    mock.assert_async().await;
    assert_eq!(
        hook_expiry.load(AtomicOrdering::SeqCst),
        1000,
        "legacy hard rebind must fire CredentialRenewed with the committed expiry"
    );
}

#[test]
fn backoff_sequence() {
    let mut b = Backoff::new();
    let d0 = b.next();
    let d1 = b.next();
    let d2 = b.next();
    let d3 = b.next();
    let d4 = b.next();

    assert!(d0 >= Duration::from_secs(1));
    assert!(d1 >= Duration::from_secs(1));
    assert!(d2 >= Duration::from_secs(1));
    assert!(d3 >= Duration::from_secs(1));
    assert!(d4 <= Duration::from_secs(75)); // 60 + 25% jitter
}

/// Hard rebind of a Package registration must re-invoke the manufacturer auth
/// provider to mint a fresh proof. The initial manufacturer nonce was consumed
/// by AIS on the first successful registration; replaying it would be
/// rejected. This locks in the re-sign behaviour added to fix replay on
/// hard rebind.
#[tokio::test]
async fn package_hard_rebind_re_signs_manufacturer_proof() {
    use actr_protocol::RegisterAuthMode;
    use std::sync::atomic::AtomicU64;

    struct CountingManufacturerProvider {
        calls: Arc<AtomicU64>,
    }
    impl crate::ManufacturerAuthProvider for CountingManufacturerProvider {
        fn sign(
            &self,
            _realm_id: u32,
            _actr_type: &actr_protocol::ActrType,
            _target: &str,
            _manifest_raw: &[u8],
        ) -> std::result::Result<crate::ManufacturerRegistrationAuth, crate::HyperError> {
            let n = self.calls.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            // Deterministic fresh proof keyed by call count. The mock AIS
            // server does not validate crypto, so the values only need to
            // differ from the stale proof saved on the original request.
            Ok(crate::ManufacturerRegistrationAuth {
                signature: vec![n as u8; 64],
                signed_at: 9_999_999_999,
                nonce: vec![n as u8; 32],
            })
        }
    }

    let calls = Arc::new(AtomicU64::new(0));
    let provider: Arc<CountingManufacturerProvider> = Arc::new(CountingManufacturerProvider {
        calls: calls.clone(),
    });

    let mut server = mockito::Server::new_async().await;
    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok(1, 9))),
    };
    let mock = server
        .mock("POST", "/reissue")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response.encode_to_vec())
        .expect(1)
        .create_async()
        .await;

    // Stale manufacturer proof (nonce 0xAA) saved on the original request — this
    // is exactly what must not be replayed to AIS.
    let request = RegisterRequest {
        actr_type: actor(1).r#type,
        realm: Realm { realm_id: 7 },
        manifest_raw: Some(Bytes::from_static(b"manifest-bytes")),
        mfr_signature: Some(Bytes::from_static(b"mfr-sig")),
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        manufacturer_auth_signature: Some(Bytes::from_static(b"stale-manufacturer-auth-sig")),
        manufacturer_auth_signed_at: Some(1),
        manufacturer_auth_nonce: Some(Bytes::from_static(&[0xAA; 32])),
        ..Default::default()
    };

    // Expired renewal token -> run_renewal_once takes the hard rebind branch.
    let session = SessionState::new(SessionSnapshot {
        actor_id: actor(1),
        credential: credential(1),
        credential_expires_at: prost_types::Timestamp {
            seconds: 10,
            nanos: 0,
        },
        turn_credential: TurnCredential {
            username: "10:actor-1".to_string(),
            password: "old".to_string(),
            expires_at: 10,
        },
        renewal_token: Bytes::from_static(b"expired-renewal-token-32bytes"),
        renewal_token_expires_at: prost_types::Timestamp {
            seconds: 1,
            nanos: 0,
        },
        generation: 1,
    });

    run_renewal_once(
        session,
        server.url(),
        None,
        RegistrationContext::Package {
            request,
            resign: Some(provider),
        },
        None,
        None,
    )
    .await
    .expect("hard rebind should commit new snapshot");

    mock.assert_async().await;
    assert_eq!(
        calls.load(AtomicOrdering::SeqCst),
        1,
        "hard rebind must re-invoke the manufacturer auth provider exactly once"
    );
}

#[test]
fn is_expired_past_and_future() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Far past → expired.
    assert!(is_expired(now - 3600));
    assert!(is_expired(0));

    // Exactly now → expired (<=).
    assert!(is_expired(now));

    // Far future → not expired.
    assert!(!is_expired(now + 3600));
}

#[tokio::test]
async fn fire_credential_renewed_invokes_callback_with_expiry() {
    use std::sync::Mutex;

    let captured: Arc<Mutex<Option<std::time::SystemTime>>> = Arc::new(Mutex::new(None));
    let cap = captured.clone();
    let cb: HookCallback = Arc::new(move |event| {
        let cap = cap.clone();
        Box::pin(async move {
            if let HookEvent::CredentialRenewed { new_expiry } = event {
                *cap.lock().unwrap() = Some(new_expiry);
            }
        })
    });

    let expires_at = prost_types::Timestamp {
        seconds: 1000,
        nanos: 0,
    };
    fire_credential_renewed(Some(&cb), &expires_at).await;

    let got = captured.lock().unwrap().take();
    let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    assert_eq!(got, Some(expected));
}

#[tokio::test]
async fn fire_credential_renewed_none_callback_is_noop() {
    // None callback is the explicit no-op contract for runtimes without observers.
    fire_credential_renewed(
        None,
        &prost_types::Timestamp {
            seconds: 5,
            nanos: 0,
        },
    )
    .await;
}

#[tokio::test]
async fn fire_credential_renewed_clamps_negative_seconds_to_zero() {
    use std::sync::Mutex;

    let captured: Arc<Mutex<Option<std::time::SystemTime>>> = Arc::new(Mutex::new(None));
    let cap = captured.clone();
    let cb: HookCallback = Arc::new(move |event| {
        let cap = cap.clone();
        Box::pin(async move {
            if let HookEvent::CredentialRenewed { new_expiry } = event {
                *cap.lock().unwrap() = Some(new_expiry);
            }
        })
    });

    // Negative seconds (malformed/epoch) must clamp to 0 → UNIX_EPOCH.
    fire_credential_renewed(
        Some(&cb),
        &prost_types::Timestamp {
            seconds: -100,
            nanos: 0,
        },
    )
    .await;
    assert_eq!(*captured.lock().unwrap(), Some(SystemTime::UNIX_EPOCH));
}
