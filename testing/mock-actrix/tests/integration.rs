//! Integration tests for `MockActrixServer`.
//!
//! These tests treat the mock as an opaque black box (HTTP/WS client driving
//! it) and make sure it speaks the wire format expected by `actr-hyper`.

use actr_hyper::{AisClient, MfrCertCache};
use actr_mock_actrix::MockActrixServer;
use actr_protocol::{ActrType, Realm, RegisterRequest, register_response};
use base64::Engine;
use ed25519_dalek::SigningKey;

fn test_register_request() -> RegisterRequest {
    RegisterRequest {
        actr_type: ActrType {
            manufacturer: "test-mfr".into(),
            name: "TestSvc".into(),
            version: "0.1.0".into(),
        },
        realm: Realm { realm_id: 42 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(b"manifest".to_vec().into()),
        mfr_signature: Some(vec![0u8; 64].into()),
        psk_token: None,
        target: Some("wasm32-unknown-unknown".into()),
    }
}

#[tokio::test]
async fn http_register_returns_valid_protobuf() {
    let server = MockActrixServer::start().await.unwrap();
    let client = AisClient::new(server.http_url());
    let resp = client
        .register_with_manifest(test_register_request())
        .await
        .expect("register should succeed");

    let ok = match resp.result {
        Some(register_response::Result::Success(ok)) => ok,
        other => panic!("expected Success, got {other:?}"),
    };

    // Sanity check: allocated actr_id + credential signed by the mock's AIS key.
    assert_eq!(ok.actr_id.r#type.manufacturer, "test-mfr");
    assert_eq!(ok.actr_id.realm.realm_id, 42);
    assert!(ok.actr_id.serial_number > 0);
    assert_eq!(ok.credential.key_id, 1);
    assert_eq!(
        ok.signing_pubkey.as_ref(),
        server.ais_signing_key().verifying_key().as_bytes()
    );
    assert!(ok.psk.is_some());
}

#[tokio::test]
async fn http_register_also_works_under_ais_prefix() {
    let server = MockActrixServer::start().await.unwrap();
    // Configs in the wild use ais_endpoint = "http://host:port/ais" — make
    // sure AisClient's `{endpoint}/register` still lands on the mock.
    let client = AisClient::new(format!("{}/ais", server.http_url()));
    let resp = client
        .register_with_manifest(test_register_request())
        .await
        .expect("register via /ais prefix should succeed");
    assert!(matches!(
        resp.result,
        Some(register_response::Result::Success(_))
    ));
}

#[tokio::test]
async fn mfr_verifying_key_roundtrip_via_mfr_cert_cache() {
    let server = MockActrixServer::start().await.unwrap();

    // Seed an MFR; the verifying key we register here is what the cache must
    // return.
    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let key_id = actr_pack::compute_key_id(&verifying_key.to_bytes());
    server.add_mfr("acme", verifying_key).await;

    let cache = MfrCertCache::new(server.http_url());
    let fetched = cache.get_or_fetch("acme", Some(&key_id)).await.unwrap();
    assert_eq!(fetched.to_bytes(), verifying_key.to_bytes());

    // Second fetch hits the in-process cache (no way to assert 0 HTTP calls
    // without intercepting, but we can at least assert it still returns the
    // same key and doesn't panic).
    let fetched2 = cache.get_or_fetch("acme", Some(&key_id)).await.unwrap();
    assert_eq!(fetched2.to_bytes(), verifying_key.to_bytes());
}

#[tokio::test]
async fn mfr_verifying_key_404_for_unregistered_mfr() {
    let server = MockActrixServer::start().await.unwrap();

    let url = format!("{}/mfr/ghost/verifying_key", server.http_url());
    let status = reqwest::get(&url).await.unwrap().status();
    assert_eq!(status, 404);
}

#[tokio::test]
async fn admin_mfr_seeds_verifying_key() {
    let server = MockActrixServer::start().await.unwrap();

    let signing_key = SigningKey::from_bytes(&[11u8; 32]);
    let pubkey_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes());

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/admin/mfr", server.http_url()))
        .json(&serde_json::json!({
            "name": "seeded",
            "pubkey_b64": pubkey_b64,
            "contact": "dev@example.com",
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "admin/mfr should succeed");

    let cache = MfrCertCache::new(server.http_url());
    let fetched = cache.get_or_fetch("seeded", None).await.unwrap();
    assert_eq!(fetched.to_bytes(), signing_key.verifying_key().to_bytes());
}

#[tokio::test]
async fn admin_realm_and_state_endpoints() {
    let server = MockActrixServer::start().await.unwrap();

    let client = reqwest::Client::new();
    client
        .post(format!("{}/admin/realms", server.http_url()))
        .json(&serde_json::json!({ "id": 99, "name": "admin-seeded" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let snapshot: serde_json::Value = client
        .get(format!("{}/admin/state", server.http_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let realms = snapshot["realms"].as_array().unwrap();
    assert!(realms.iter().any(|v| v.as_u64() == Some(99)));
}

#[tokio::test]
async fn publish_flow_via_nonce_and_publish() {
    let server = MockActrixServer::start().await.unwrap();

    let client = reqwest::Client::new();

    // Step 1: get a nonce.
    let nonce_resp: serde_json::Value = client
        .post(format!("{}/mfr/pkg/nonce", server.http_url()))
        .json(&serde_json::json!({ "manufacturer": "acme" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nonce = nonce_resp["nonce"].as_str().unwrap().to_string();

    // Step 2: publish with that nonce.
    let publish: serde_json::Value = client
        .post(format!("{}/mfr/pkg/publish", server.http_url()))
        .json(&serde_json::json!({
            "manufacturer": "acme",
            "name": "EchoSvc",
            "version": "0.1.0",
            "target": "wasm32-unknown-unknown",
            "manifest": "edition = 1\n",
            "signature": "sig",
            "proto_files": null,
            "nonce": nonce,
            "nonce_sig": "anything",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(publish["type_str"], "acme:EchoSvc:0.1.0");
    assert_eq!(publish["status"], "active");

    // Step 3: same nonce cannot be reused.
    let reuse = client
        .post(format!("{}/mfr/pkg/publish", server.http_url()))
        .json(&serde_json::json!({
            "manufacturer": "acme",
            "name": "EchoSvc",
            "version": "0.1.0",
            "target": "wasm32-unknown-unknown",
            "manifest": "edition = 1\n",
            "signature": "sig",
            "proto_files": null,
            "nonce": nonce,
            "nonce_sig": "anything",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(reuse.status(), 400, "reused nonce must be rejected");
}
