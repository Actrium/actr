//! Integration tests: PSK Bootstrap two-phase full flow
//!
//! Covered scenarios:
//! 1. First registration: no PSK -> manifest auth -> AIS issues credential + PSK -> stored
//! 2. PSK renewal: valid PSK -> PSK auth -> AIS issues new credential (no new PSK)
//! 3. PSK expired: PSK expired -> fallback to manifest auth -> AIS issues credential + new PSK
//! 4. PSK update: after first registration, register again -> uses newly obtained PSK
//! 5. AIS error: AIS returns error -> correctly propagates HyperError

use std::time::{SystemTime, UNIX_EPOCH};

use actr_hyper::{ActorStore, Hyper, HyperConfig, HyperError, PackageManifest, TrustMode};
use ed25519_dalek::SigningKey;
use prost::Message;
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn dev_config(dir: &TempDir) -> HyperConfig {
    let signing_key = SigningKey::generate(&mut OsRng);
    let pubkey = signing_key.verifying_key().to_bytes().to_vec();
    HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Development {
        self_signed_pubkey: pubkey,
    })
}

fn fake_manifest() -> PackageManifest {
    PackageManifest {
        manufacturer: "test-mfr".to_string(),
        actr_name: "TestActor".to_string(),
        version: "0.1.0".to_string(),
        binary_hash: [0u8; 32],
        capabilities: vec![],
        signature: vec![0u8; 64],
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Build valid RegisterResponse protobuf bytes
fn make_register_response(with_psk: bool, psk_bytes: Option<&[u8]>) -> Vec<u8> {
    use actr_protocol::{
        AIdCredential, ActrId, ActrType, IdentityClaims, Realm, RegisterResponse, TurnCredential,
        register_response,
    };

    let claims = IdentityClaims {
        realm_id: 1,
        actor_id: "test-actor-id".to_string(),
        expires_at: u64::MAX,
    };
    let credential = AIdCredential {
        key_id: 1,
        claims: claims.encode_to_vec().into(),
        signature: vec![0u8; 64].into(),
    };
    let actr_id = ActrId {
        realm: Realm { realm_id: 1 },
        serial_number: 42,
        r#type: ActrType {
            manufacturer: "test-mfr".to_string(),
            name: "TestActor".to_string(),
            version: "0.1.0".to_string(),
        },
    };
    let turn = TurnCredential {
        username: "u".to_string(),
        password: "p".to_string(),
        expires_at: u64::MAX,
    };

    let mut ok = register_response::RegisterOk {
        actr_id,
        credential,
        turn_credential: turn,
        credential_expires_at: None,
        signaling_heartbeat_interval_secs: 30,
        signing_pubkey: vec![0u8; 32].into(),
        signing_key_id: 1,
        psk: None,
        psk_expires_at: None,
    };

    if with_psk {
        let psk = psk_bytes.unwrap_or(b"server-generated-psk");
        ok.psk = Some(psk.to_vec().into());
        ok.psk_expires_at = Some((now_secs() + 86400) as i64); // expires in 24 hours
    }

    RegisterResponse {
        result: Some(register_response::Result::Success(ok)),
    }
    .encode_to_vec()
}

fn make_error_response(code: u32, message: &str) -> Vec<u8> {
    use actr_protocol::{ErrorResponse, RegisterResponse, register_response};
    RegisterResponse {
        result: Some(register_response::Result::Error(ErrorResponse {
            code,
            message: message.to_string(),
        })),
    }
    .encode_to_vec()
}

// ─── Test cases ─────────────────────────────────────────────────────────────────

/// Scenario 1: first registration (no PSK in ActorStore) -> manifest auth -> receives credential + PSK -> stored
#[tokio::test]
async fn first_registration_uses_manifest_auth_and_stores_psk() {
    let psk = b"initial-psk-token";
    let resp_body = make_register_response(true, Some(psk));

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .expect(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    let credential = hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    mock.assert_async().await;
    assert!(!credential.is_empty(), "credential should not be empty");

    // PSK should be written to ActorStore
    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();
    let stored_psk = store.kv_get("hyper:psk:token").await.unwrap();
    assert_eq!(stored_psk, Some(psk.to_vec()), "PSK should be persisted");

    // expires_at should also be valid
    let expires = store.kv_get("hyper:psk:expires_at").await.unwrap();
    assert!(expires.is_some(), "PSK expires_at should be persisted");
    let expires_secs = u64::from_le_bytes(expires.unwrap().try_into().unwrap());
    assert!(expires_secs > now_secs(), "PSK should not be expired");
}

/// Scenario 2: valid PSK -> PSK auth -> only one /register call -> no new PSK issued
#[tokio::test]
async fn valid_psk_uses_psk_auth_without_new_psk() {
    let resp_body = make_register_response(false, None);

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .expect(1) // called exactly once
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    // Pre-populate valid PSK
    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();
    let valid_psk = b"existing-valid-psk";
    store.kv_set("hyper:psk:token", valid_psk).await.unwrap();
    store
        .kv_set("hyper:psk:expires_at", &(now_secs() + 3600).to_le_bytes())
        .await
        .unwrap();

    let credential = hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    mock.assert_async().await;
    assert!(!credential.is_empty());

    // PSK should remain unchanged (no new PSK issued)
    let stored = store.kv_get("hyper:psk:token").await.unwrap();
    assert_eq!(
        stored,
        Some(valid_psk.to_vec()),
        "PSK should remain unchanged"
    );
}

/// Scenario 3: PSK expired -> fallback to manifest auth -> receives new PSK
#[tokio::test]
async fn expired_psk_falls_back_to_manifest_and_receives_new_psk() {
    let new_psk = b"renewed-psk-after-expiry";
    let resp_body = make_register_response(true, Some(new_psk));

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .expect(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    // Pre-populate expired PSK (10 seconds ago)
    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();
    store
        .kv_set("hyper:psk:token", b"old-expired-psk")
        .await
        .unwrap();
    store
        .kv_set(
            "hyper:psk:expires_at",
            &now_secs().saturating_sub(10).to_le_bytes(),
        )
        .await
        .unwrap();

    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    mock.assert_async().await;

    // New PSK should overwrite the old one
    let stored = store.kv_get("hyper:psk:token").await.unwrap();
    assert_eq!(
        stored,
        Some(new_psk.to_vec()),
        "should receive and store new PSK after expiry"
    );
}

/// Scenario 4: two sequential registrations (first + renewal) -> first uses manifest, second uses PSK
#[tokio::test]
async fn sequential_registrations_switch_from_manifest_to_psk() {
    let psk = b"sequential-psk";
    let first_resp = make_register_response(true, Some(psk));
    let second_resp = make_register_response(false, None);

    let mut server = mockito::Server::new_async().await;

    // First: returns PSK
    let mock1 = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(first_resp)
        .expect(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    // First registration (manifest auth)
    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();
    mock1.assert_async().await;

    // Second: returns credential, no new PSK
    let mock2 = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(second_resp)
        .expect(1)
        .create_async()
        .await;

    // Second registration (PSK auth, since first already stored PSK)
    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();
    mock2.assert_async().await;
}

/// Scenario 5: AIS returns 403 -> propagates as HyperError::AisBootstrapFailed
#[tokio::test]
async fn ais_error_propagates_as_bootstrap_failed() {
    let error_resp = make_error_response(403u32, "manufacturer not registered");

    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(error_resp)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();

    let result = hyper
        .bootstrap_credential(&fake_manifest(), &server.url(), 1)
        .await;

    assert!(
        matches!(result, Err(HyperError::AisBootstrapFailed(_))),
        "AIS error should propagate as AisBootstrapFailed, got: {result:?}"
    );
}

/// Scenario 6: AIS unreachable (connection refused) -> propagates as HyperError::AisBootstrapFailed (network error)
#[tokio::test]
async fn ais_unreachable_propagates_error() {
    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();

    // Use invalid port
    let result = hyper
        .bootstrap_credential(&fake_manifest(), "http://127.0.0.1:19999", 1)
        .await;

    assert!(
        result.is_err(),
        "should return error when AIS is unreachable, got: {result:?}"
    );
}

/// PSK and signing_pubkey should both be persisted after first registration
#[tokio::test]
async fn first_registration_persists_signing_pubkey() {
    let resp_body = make_register_response(true, Some(b"my-psk"));

    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();

    let pubkey = store.kv_get("hyper:ais:signing_pubkey").await.unwrap();
    assert!(pubkey.is_some(), "signing_pubkey should be persisted");
    assert_eq!(
        pubkey.unwrap().len(),
        32,
        "Ed25519 pubkey should be 32 bytes"
    );

    let key_id = store.kv_get("hyper:ais:signing_key_id").await.unwrap();
    assert!(key_id.is_some(), "signing_key_id should be persisted");
}
