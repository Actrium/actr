use base64::Engine as _;
use nonce_auth::{CredentialBuilder, NonceCredential, storage::MemoryStorage};
use reqwest::StatusCode;
use serde_json::Value;
use signer::{
    GenerateSigningKeyRequest, GenerateSigningKeyResponse, SignerServiceConfig, create_router,
    create_signer_state, types::SignRequest,
};
use tempfile::TempDir;
use tokio::net::TcpListener;

struct TestServer {
    base_url: String,
    _temp_dir: TempDir,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn start_test_server(psk: &str) -> TestServer {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let config = SignerServiceConfig::default();
    let state = create_signer_state(&config, MemoryStorage::new(), psk, temp_dir.path())
        .await
        .expect("Failed to create KS state");
    let app = create_router(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind listener");
    let addr = listener.local_addr().expect("Failed to read bound addr");
    let base_url = format!("http://{addr}");

    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("KS test server exited unexpectedly");
    });

    TestServer {
        base_url,
        _temp_dir: temp_dir,
        handle,
    }
}

fn sign_request(psk: &str, payload: &str) -> NonceCredential {
    CredentialBuilder::new(psk.as_bytes())
        .sign(payload.as_bytes())
        .expect("Failed to sign nonce credential")
}

async fn generate_signing_key_via_http(
    client: &reqwest::Client,
    base_url: &str,
    psk: &str,
) -> GenerateSigningKeyResponse {
    let credential = sign_request(psk, "generate_signing_key");
    let req = GenerateSigningKeyRequest { credential };
    let resp = client
        .post(format!("{base_url}/generate-signing-key"))
        .json(&req)
        .send()
        .await
        .expect("generate signing key request failed");
    assert_eq!(resp.status(), StatusCode::OK);
    resp.json()
        .await
        .expect("generate signing key response should parse")
}

#[tokio::test]
async fn test_http_health_and_key_lifecycle() {
    let psk = "test-ks-psk";
    let server = start_test_server(psk).await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("{}/health", server.base_url))
        .send()
        .await
        .expect("health request failed");
    assert_eq!(health.status(), StatusCode::OK);
    let health_json: Value = health.json().await.expect("health body should be json");
    assert_eq!(health_json["status"], "healthy");

    let credential = sign_request(psk, "generate_signing_key");
    let generate_req = GenerateSigningKeyRequest { credential };
    let generated = client
        .post(format!("{}/generate-signing-key", server.base_url))
        .json(&generate_req)
        .send()
        .await
        .expect("generate signing key request failed");
    assert_eq!(generated.status(), StatusCode::OK);

    let generated: GenerateSigningKeyResponse = generated
        .json()
        .await
        .expect("generate signing key response should parse");
    assert!(generated.key_id > 0);
    assert!(generated.expires_at > 0);

    // verifying key should be 32 bytes
    let vk_bytes = base64::engine::general_purpose::STANDARD
        .decode(generated.verifying_key.as_bytes())
        .expect("verifying key should be valid base64");
    assert_eq!(vk_bytes.len(), 32, "Ed25519 verifying key must be 32 bytes");

    // test sign endpoint
    let message = b"hello, world";
    let sign_payload = format!("sign:{}", generated.key_id);
    let sign_cred = sign_request(psk, &sign_payload);
    let sign_req = SignRequest {
        key_id: generated.key_id,
        message: message.to_vec(),
        credential: sign_cred,
    };
    let sign_resp = client
        .post(format!("{}/sign/{}", server.base_url, generated.key_id))
        .json(&sign_req)
        .send()
        .await
        .expect("sign request failed");
    assert_eq!(sign_resp.status(), StatusCode::OK);
    let sign_body: Value = sign_resp.json().await.expect("sign response should parse");
    // signature field should be a byte array (JSON array of numbers or base64)
    assert!(sign_body["signature"].is_array() || sign_body["signature"].is_string());
}

#[tokio::test]
async fn test_http_generate_rejects_invalid_signature() {
    let psk = "test-ks-psk";
    let server = start_test_server(psk).await;
    let client = reqwest::Client::new();

    let invalid_credential = sign_request(psk, "not-generate-signing-key");
    let req = GenerateSigningKeyRequest {
        credential: invalid_credential,
    };
    let response = client
        .post(format!("{}/generate-signing-key", server.base_url))
        .json(&req)
        .send()
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_http_generate_replay_is_rejected() {
    let psk = "test-ks-psk";
    let server = start_test_server(psk).await;
    let client = reqwest::Client::new();

    let credential = sign_request(psk, "generate_signing_key");
    let req_body = GenerateSigningKeyRequest {
        credential: credential.clone(),
    };

    let url = format!("{}/generate-signing-key", server.base_url);
    let first = client.post(&url).json(&req_body).send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = client.post(&url).json(&req_body).send().await.unwrap();
    // replay should be rejected
    assert_eq!(second.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_http_generate_timestamp_out_of_window() {
    let psk = "test-ks-psk";
    let server = start_test_server(psk).await;
    let client = reqwest::Client::new();

    let mut credential = sign_request(psk, "generate_signing_key");
    // push timestamp far into the past to exceed default window
    credential.timestamp = 0;
    let req_body = GenerateSigningKeyRequest { credential };

    let url = format!("{}/generate-signing-key", server.base_url);
    let resp = client.post(&url).json(&req_body).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_http_sign_rejects_invalid_signature() {
    let psk = "test-ks-psk";
    let server = start_test_server(psk).await;
    let client = reqwest::Client::new();

    let generated = generate_signing_key_via_http(&client, &server.base_url, psk).await;

    // use wrong payload for credential
    let invalid_cred = sign_request(psk, "wrong-payload");
    let sign_req = SignRequest {
        key_id: generated.key_id,
        message: b"test".to_vec(),
        credential: invalid_cred,
    };

    let resp = client
        .post(format!("{}/sign/{}", server.base_url, generated.key_id))
        .json(&sign_req)
        .send()
        .await
        .expect("sign request should complete");

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_http_sign_nonexistent_key_returns_not_found() {
    let psk = "test-ks-psk";
    let server = start_test_server(psk).await;
    let client = reqwest::Client::new();

    let missing_key_id = 99_999_u32;
    let sign_payload = format!("sign:{missing_key_id}");
    let sign_cred = sign_request(psk, &sign_payload);
    let sign_req = SignRequest {
        key_id: missing_key_id,
        message: b"test".to_vec(),
        credential: sign_cred,
    };

    let resp = client
        .post(format!("{}/sign/{missing_key_id}", server.base_url))
        .json(&sign_req)
        .send()
        .await
        .expect("sign request should complete");

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
