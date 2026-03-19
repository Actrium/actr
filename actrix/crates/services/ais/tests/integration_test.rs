//! AIS 集成测试（自举式）
//!
//! 在测试进程内启动临时 Signer gRPC 服务，验证 AIS 的签发与校验链路。

use actr_protocol::{ActrType, Realm, RegisterRequest, register_response};
use ais::issuer::{AIdIssuer, IssuerConfig};
use ais::signer_client_wrapper::create_signer_client;
use nonce_auth::storage::MemoryStorage;
use platform::aid::credential::validator::AIdCredentialValidator;
use platform::config::signer::SignerClientConfig;
use serial_test::serial;
use signer::{GrpcClient, GrpcClientConfig, KeyStorage, SignerServiceConfig, create_grpc_service};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tonic::transport::Server;

struct TestEnv {
    issuer_temp_dir: TempDir,
    _signer_temp_dir: TempDir,
    signer_handle: Option<JoinHandle<()>>,
    signer_shutdown_tx: Option<oneshot::Sender<()>>,
    signer_config: SignerClientConfig,
    shared_key: String,
}

impl TestEnv {
    async fn shutdown_signer(&mut self) {
        if let Some(tx) = self.signer_shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.signer_handle.take() {
            let _ = handle.await;
        }
    }
}

async fn start_embedded_signer(
    psk: &str,
    sqlite_path: &Path,
) -> (String, JoinHandle<()>, oneshot::Sender<()>) {
    let service_config = SignerServiceConfig::default();
    let storage = KeyStorage::from_config(
        &service_config.storage,
        signer::KeyEncryptor::no_encryption(),
        sqlite_path,
    )
    .await
    .expect("Failed to create Signer storage");

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral port");
    let addr = listener.local_addr().expect("Failed to get local addr");
    drop(listener);

    let service = create_grpc_service(
        storage,
        MemoryStorage::new(),
        psk.to_string(),
        service_config.tolerance_seconds,
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
        if let Err(err) = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async move {
                let _ = shutdown_rx.await;
            })
            .await
        {
            panic!("Embedded Signer server failed: {err}");
        }
    });

    let endpoint = format!("http://{addr}");

    // Wait until gRPC health check is reachable.
    let mut last_error = String::new();
    for _ in 0..40 {
        let cfg = GrpcClientConfig {
            endpoint: endpoint.clone(),
            actrix_shared_key: psk.to_string(),
            timeout_seconds: 2,
            enable_tls: false,
            tls_domain: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
        };

        match GrpcClient::new(&cfg).await {
            Ok(mut client) => match client.health_check().await {
                Ok(status) if status == "healthy" => return (endpoint, handle, shutdown_tx),
                Ok(status) => last_error = format!("unexpected Signer health status: {status}"),
                Err(err) => last_error = err.to_string(),
            },
            Err(err) => last_error = err.to_string(),
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("Embedded Signer did not become healthy in time: {last_error}");
}

async fn setup_test_environment() -> TestEnv {
    let issuer_temp_dir = TempDir::new().expect("Failed to create issuer temp dir");
    let signer_temp_dir = TempDir::new().expect("Failed to create signer temp dir");
    let shared_key = "test-psk-key".to_string();
    let (endpoint, signer_handle, signer_shutdown_tx) =
        start_embedded_signer(&shared_key, signer_temp_dir.path()).await;

    let signer_config = SignerClientConfig {
        endpoint,
        timeout_seconds: 10,
        enable_tls: false,
        tls_domain: None,
        ca_cert: None,
        client_cert: None,
        client_key: None,
    };

    TestEnv {
        issuer_temp_dir,
        _signer_temp_dir: signer_temp_dir,
        signer_handle: Some(signer_handle),
        signer_shutdown_tx: Some(signer_shutdown_tx),
        signer_config,
        shared_key,
    }
}

fn default_issuer_config(temp_dir: &TempDir) -> IssuerConfig {
    IssuerConfig {
        token_ttl_secs: 3600,
        signaling_heartbeat_interval_secs: 30,
        key_refresh_interval_secs: 3600,
        key_storage_file: temp_dir.path().join("issuer_keys.db"),
        enable_periodic_rotation: false,
        key_rotation_interval_secs: 86400,
        turn_secret: "test-turn-secret".to_string(),
        sqlite_path: temp_dir.path().to_path_buf(),
    }
}

#[tokio::test]
#[serial]
async fn test_end_to_end_credential_flow() {
    let env = setup_test_environment().await;

    AIdCredentialValidator::init(env.issuer_temp_dir.path())
        .await
        .expect("Failed to initialize validator");

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("Failed to create Signer gRPC client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("Failed to create issuer");

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "acme".to_string(),
            name: "test-device".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_json: None,
        mfr_signature: None,
        psk_token: None,
    };

    let response = issuer
        .issue_credential(&request)
        .await
        .expect("Failed to issue credential");

    let register_ok = match response.result.expect("Response should contain result") {
        register_response::Result::Success(ok) => ok,
        register_response::Result::Error(err) => panic!("Expected success but got error: {err:?}"),
    };

    assert!(
        !register_ok.turn_credential.username.is_empty(),
        "TURN credential should be present"
    );
    assert!(
        register_ok.credential_expires_at.is_some(),
        "Credential expiry should be present"
    );
    assert_eq!(register_ok.actr_id.realm.realm_id, 1001);
    assert!(register_ok.actr_id.serial_number > 0);

    let (claims, _) = AIdCredentialValidator::check(&register_ok.credential, 1001)
        .await
        .expect("Token validation should succeed");
    assert_eq!(claims.realm_id, 1001);
    assert!(
        !claims.actor_id.is_empty(),
        "Actor ID should be present in claims"
    );
    assert!(
        claims.actor_id.contains(':') && claims.actor_id.contains('@'),
        "Actor ID format should include manufacturer/name and serial/realm separators"
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();
    assert!(claims.expires_at > now);
    assert!(claims.expires_at <= now + 3600);

    let wrong_realm_result = AIdCredentialValidator::check(&register_ok.credential, 9999).await;
    assert!(
        wrong_realm_result.is_err(),
        "Validation should fail with mismatched realm_id"
    );

    // Issue and validate multiple credentials to verify stability.
    for idx in 0..5 {
        let req = RegisterRequest {
            actr_type: ActrType {
                manufacturer: "acme".to_string(),
                name: format!("test-device-{idx}"),
                version: "1.0.0".to_string(),
            },
            realm: Realm { realm_id: 1001 },
            service_spec: None,
            acl: None,
            service: None,
            ws_address: None,
            manifest_json: None,
            mfr_signature: None,
            psk_token: None,
        };

        let rsp = issuer
            .issue_credential(&req)
            .await
            .unwrap_or_else(|e| panic!("Failed to issue credential {idx}: {e}"));
        let ok = match rsp.result.expect("Response should contain result") {
            register_response::Result::Success(ok) => ok,
            register_response::Result::Error(err) => {
                panic!("Expected success for token {idx}, got error: {err:?}")
            }
        };

        let (claims, _) = AIdCredentialValidator::check(&ok.credential, 1001)
            .await
            .unwrap_or_else(|e| panic!("Failed to validate credential {idx}: {e}"));
        assert_eq!(claims.realm_id, 1001);
    }
}

#[tokio::test]
#[serial]
async fn test_issuer_health_checks() {
    let env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("Failed to create Signer gRPC client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("Failed to create issuer");

    issuer
        .check_database_health()
        .await
        .expect("Database health check should pass");
    issuer
        .check_key_cache_health()
        .await
        .expect("Key cache health check should pass");
}

#[tokio::test]
#[serial]
async fn test_issuer_rotate_key_updates_current_key() {
    let env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("Failed to create Signer gRPC client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("Failed to create issuer");

    let key_before = issuer
        .get_current_key_id()
        .await
        .expect("current key before rotate");
    let rotated = issuer.rotate_key().await.expect("rotate key");
    let key_after = issuer
        .get_current_key_id()
        .await
        .expect("current key after rotate");

    assert_ne!(key_before, rotated, "rotate_key should change key id");
    assert_eq!(key_after, rotated, "current key should match rotated key");

    issuer
        .check_ks_health()
        .await
        .expect("Signer health should still pass after rotation");
    let cache = issuer
        .check_key_cache_health()
        .await
        .expect("key cache should remain healthy");
    assert_eq!(cache.key_id, rotated);
}

#[tokio::test]
#[serial]
async fn test_issuer_creation_fails_with_wrong_shared_key() {
    let env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, "wrong-shared-key")
        .await
        .expect("gRPC channel creation should succeed even with wrong secret");

    // With lazy Signer connection, issuer creation succeeds — auth failure happens on first use
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer creation succeeds with lazy Signer connection");

    // Credential issuance should return an error response due to wrong shared key
    let request = actr_protocol::RegisterRequest {
        actr_type: actr_protocol::ActrType {
            manufacturer: "acme".to_string(),
            name: "bad-key-test".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: actr_protocol::Realm { realm_id: 1 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_json: None,
        mfr_signature: None,
        psk_token: None,
    };
    let resp = issuer
        .issue_credential(&request)
        .await
        .expect("issue_credential returns Ok wrapping the error");
    assert!(
        matches!(
            resp.result,
            Some(actr_protocol::register_response::Result::Error(_))
        ),
        "expected error result in register response with wrong shared key, got {:?}",
        resp.result
    );
}

#[tokio::test]
#[serial]
async fn test_issuer_check_ks_health_fails_after_ks_shutdown() {
    let mut env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("Failed to create Signer gRPC client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("Failed to create issuer");

    env.shutdown_signer().await;

    for _ in 0..40 {
        match issuer.check_ks_health().await {
            Ok(()) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    msg.contains("Signer service unhealthy")
                        || msg.contains("Failed")
                        || msg.contains("transport")
                        || msg.contains("unavailable")
                        || msg.contains("connection"),
                    "unexpected Signer health error after shutdown: {msg}"
                );
                return;
            }
        }
    }

    panic!("issuer Signer health should fail after embedded Signer shutdown");
}

#[tokio::test]
#[serial]
async fn test_issuer_rotate_key_fails_when_ks_is_unavailable() {
    let mut env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("Failed to create Signer gRPC client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("Failed to create issuer");

    env.shutdown_signer().await;

    for _ in 0..40 {
        match issuer.rotate_key().await {
            Ok(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    msg.contains("KS unavailable")
                        || msg.contains("Failed")
                        || msg.contains("transport")
                        || msg.contains("connection"),
                    "unexpected rotate_key error after KS shutdown: {msg}"
                );
                return;
            }
        }
    }

    panic!("rotate_key should fail after embedded KS shutdown");
}
