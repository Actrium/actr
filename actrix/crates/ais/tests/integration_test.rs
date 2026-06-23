//! AIS 集成测试（自举式）
//!
//! 在测试进程内启动临时 Signer gRPC 服务，验证 AIS 的签发与校验链路。

use actr_protocol::{
    ActrType, Realm, RegisterAuthMode, RegisterRequest, RegisterResponse, register_response,
};
use ais::signer_client_wrapper::create_signer_client;
use ais::{
    handlers::{AISState, create_router},
    issuer::{AIdIssuer, IssuerConfig},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::Engine as _;
use nonce_auth::storage::MemoryStorage;
use platform::aid::credential::validator::AIdCredentialValidator;
use platform::config::signer::SignerClientConfig;
use platform::realm::{REALM_SECRET_HEADER, RealmSecretCheck, hash_realm_secret};
use prost::Message;
use serial_test::serial;
use signer::{GrpcClient, GrpcClientConfig, KeyStorage, SignerServiceConfig, create_grpc_service};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tonic::transport::Server;
use tower::ServiceExt;

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
    use std::sync::OnceLock;
    static DB_DIR: OnceLock<TempDir> = OnceLock::new();

    let issuer_temp_dir = TempDir::new().expect("Failed to create issuer temp dir");
    let signer_temp_dir = TempDir::new().expect("Failed to create signer temp dir");
    let shared_key = "test-psk-key".to_string();
    let (endpoint, signer_handle, signer_shutdown_tx) =
        start_embedded_signer(&shared_key, signer_temp_dir.path()).await;

    // Initialize the global database once with a persistent temp dir.
    // The OnceLock ensures the TempDir (and its SQLite file) lives for the
    // entire process, avoiding "unable to open database file" in serial tests.
    let db_dir = DB_DIR.get_or_init(|| TempDir::new().expect("Failed to create DB temp dir"));
    if !platform::storage::db::is_database_initialized() {
        platform::storage::db::set_db_path(db_dir.path())
            .await
            .expect("Failed to initialize test database");
    }

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

fn linked_register_request() -> RegisterRequest {
    RegisterRequest {
        actr_type: ActrType {
            manufacturer: "linked-src".to_string(),
            name: "source-workload".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: None,
        mfr_signature: None,
        psk_token: None,
        target: None,
        auth_mode: Some(RegisterAuthMode::Linked as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    }
}

async fn create_test_router(env: &TestEnv) -> Router {
    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    create_router(AISState::new(issuer))
}

async fn seed_realm(realm_id: u32, name: &str, secret: Option<&str>) {
    let pool = platform::storage::db::get_database().get_pool();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let secret_current = secret.map(hash_realm_secret).unwrap_or_default();

    sqlx::query(
        "INSERT OR REPLACE INTO realm (id, name, status, enabled, created_at, secret_current)
         VALUES (?, ?, 'Active', 1, ?, ?)",
    )
    .bind(realm_id as i64)
    .bind(name)
    .bind(now)
    .bind(secret_current)
    .execute(pool)
    .await
    .expect("seed realm");
}

fn test_registry_manifest(manufacturer: &str, name: &str, version: &str, target: &str) -> Vec<u8> {
    format!(
        r#"manufacturer = "{manufacturer}"
name = "{name}"
version = "{version}"

[binary]
path = "bin/actor.wasm"
target = "{target}"
hash = "0000000000000000000000000000000000000000000000000000000000000000"
"#,
    )
    .into_bytes()
}

async fn seed_active_package(
    manufacturer: &str,
    name: &str,
    version: &str,
    target: &str,
) -> Vec<u8> {
    let pool = platform::storage::db::get_database().get_pool();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let manifest = test_registry_manifest(manufacturer, name, version, target);
    let manifest_str = std::str::from_utf8(&manifest).expect("test manifest is UTF-8");

    sqlx::query(
        "INSERT OR IGNORE INTO mfr (name, public_key, status, created_at)
         VALUES (?, '', 'active', ?)",
    )
    .bind(manufacturer)
    .bind(now)
    .execute(pool)
    .await
    .expect("seed mfr");

    let mfr_id: i64 = sqlx::query_scalar("SELECT id FROM mfr WHERE name = ?")
        .bind(manufacturer)
        .fetch_one(pool)
        .await
        .expect("get mfr id");

    sqlx::query(
        "INSERT OR REPLACE INTO mfr_package
         (mfr_id, manufacturer, name, version, type_str, target, manifest, signature, status, published_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, '', 'active', ?)",
    )
    .bind(mfr_id)
    .bind(manufacturer)
    .bind(name)
    .bind(version)
    .bind(format!("{manufacturer}:{name}:{version}"))
    .bind(target)
    .bind(manifest_str)
    .bind(now)
    .execute(pool)
    .await
    .expect("seed mfr package");

    manifest
}

async fn post_register(
    app: Router,
    request: RegisterRequest,
    realm_secret: Option<&str>,
) -> RegisterResponse {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/protobuf")
        .header("x-real-ip", "127.0.0.1");
    if let Some(secret) = realm_secret {
        builder = builder.header(REALM_SECRET_HEADER, secret);
    }

    let response = app
        .oneshot(builder.body(Body::from(request.encode_to_vec())).unwrap())
        .await
        .expect("register route response");
    let status = response.status();

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("register response body");
    assert_eq!(
        status,
        StatusCode::OK,
        "unexpected /register status {status}: {}",
        String::from_utf8_lossy(&body)
    );
    RegisterResponse::decode(body).expect("decode register response")
}

#[tokio::test]
#[serial]
async fn test_register_route_linked_with_realm_secret_succeeds() {
    let env = setup_test_environment().await;
    let realm_secret = "linked-http-route-secret";
    let realm_id = 22001;
    seed_realm(realm_id, "linked-http-route", Some(realm_secret)).await;
    let app = create_test_router(&env).await;

    let mut request = linked_register_request();
    request.realm = Realm { realm_id };
    let response = post_register(app, request, Some(realm_secret)).await;

    match response.result.expect("result") {
        register_response::Result::Success(ok) => {
            assert_eq!(ok.actr_id.realm.realm_id, realm_id);
            assert_eq!(ok.actr_id.r#type.name, "source-workload");
        }
        register_response::Result::Error(err) => {
            panic!("linked /register with realm secret should succeed: {err:?}")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_register_route_package_with_mfr_identity_succeeds() {
    let env = setup_test_environment().await;
    let realm_id = 22002;
    seed_realm(realm_id, "package-http-route", None).await;
    let manifest_raw =
        seed_active_package("httppkg", "PackagedService", "1.0.0", "wasm32-wasip1").await;
    let app = create_test_router(&env).await;

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "httppkg".to_string(),
            name: "PackagedService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_raw)),
        mfr_signature: None,
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    let response = post_register(app, request, None).await;

    match response.result.expect("result") {
        register_response::Result::Success(ok) => {
            assert_eq!(ok.actr_id.realm.realm_id, realm_id);
            assert_eq!(ok.actr_id.r#type.manufacturer, "httppkg");
        }
        register_response::Result::Error(err) => {
            panic!("package /register with MFR identity should succeed: {err:?}")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_register_route_unspecified_auth_mode_uses_package_identity() {
    let env = setup_test_environment().await;
    let realm_id = 22004;
    seed_realm(realm_id, "unspecified-http-route", None).await;
    let manifest_raw =
        seed_active_package("legacyhttp", "LegacyService", "1.0.0", "wasm32-wasip1").await;
    let app = create_test_router(&env).await;

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "legacyhttp".to_string(),
            name: "LegacyService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_raw)),
        mfr_signature: None,
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: None,
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    let response = post_register(app, request, None).await;

    match response.result.expect("result") {
        register_response::Result::Success(ok) => {
            assert_eq!(ok.actr_id.realm.realm_id, realm_id);
            assert_eq!(ok.actr_id.r#type.manufacturer, "legacyhttp");
        }
        register_response::Result::Error(err) => {
            panic!("omitted auth_mode should remain package-compatible: {err:?}")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_register_route_published_package_without_manifest_raw_is_rejected() {
    let env = setup_test_environment().await;
    let realm_id = 22005;
    seed_realm(realm_id, "published-package-missing-manifest", None).await;
    seed_active_package(
        "missingmanifest",
        "PublishedService",
        "1.0.0",
        "wasm32-wasip1",
    )
    .await;
    let app = create_test_router(&env).await;

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "missingmanifest".to_string(),
            name: "PublishedService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: None,
        mfr_signature: None,
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    let response = post_register(app, request, None).await;

    match response.result.expect("result") {
        register_response::Result::Error(err) => {
            assert_eq!(err.code, 400);
        }
        register_response::Result::Success(_) => {
            panic!("published package registration without manifest_raw should be rejected")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_register_route_published_package_without_target_is_rejected() {
    let env = setup_test_environment().await;
    let realm_id = 22006;
    seed_realm(realm_id, "published-package-missing-target", None).await;
    let manifest_raw = seed_active_package(
        "missingtarget",
        "PublishedService",
        "1.0.0",
        "wasm32-wasip1",
    )
    .await;
    let app = create_test_router(&env).await;

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "missingtarget".to_string(),
            name: "PublishedService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_raw)),
        mfr_signature: None,
        psk_token: None,
        target: None,
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    let response = post_register(app, request, None).await;

    match response.result.expect("result") {
        register_response::Result::Error(err) => {
            assert_eq!(err.code, 400);
        }
        register_response::Result::Success(_) => {
            panic!("published package registration without target should be rejected")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_published_package_manifest_hash_mismatch_does_not_fall_back_to_path2() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let env = setup_test_environment().await;
    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    seed_realm(22007, "published-package-manifest-mismatch", None).await;
    let pool = platform::storage::db::get_database().get_pool();
    let key = SigningKey::generate(&mut OsRng);
    let (_mfr_id, _key_id) = seed_mfr_with_key(pool, "publishedmismatch", &key).await;
    seed_active_package(
        "publishedmismatch",
        "MismatchService",
        "1.0.0",
        "wasm32-wasip1",
    )
    .await;

    // This manifest is validly signed and would pass Path 2, but it is not the
    // same manifest registered in mfr_package. A published package hash mismatch
    // must be rejected immediately instead of falling through to Path 2.
    let (manifest_bytes, sig_bytes) = build_signed_manifest(
        &key,
        "publishedmismatch",
        "MismatchService",
        "1.0.0",
        "wasm32-wasip1",
    );
    let mut request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "publishedmismatch".to_string(),
            name: "MismatchService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 22007 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_bytes.clone())),
        mfr_signature: Some(prost::bytes::Bytes::from(sig_bytes)),
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    sign_runner_request_at(
        &mut request,
        &key,
        &manifest_bytes,
        test_unix_now_secs(),
        vec![0x55; 32],
    );

    let response = issuer.issue_credential(&request).await.expect("issue");
    match response.result.expect("result") {
        register_response::Result::Error(err) => {
            assert_eq!(err.code, 403);
            assert!(err.message.contains("manufacturer not verified"));
        }
        register_response::Result::Success(_) => {
            panic!("published package manifest hash mismatch should not fall back to Path 2")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_register_route_package_without_mfr_identity_is_still_rejected() {
    let env = setup_test_environment().await;
    let realm_id = 22003;
    seed_realm(realm_id, "package-http-route-missing-mfr", None).await;
    let app = create_test_router(&env).await;

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "missing-http-mfr".to_string(),
            name: "PackagedService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(test_registry_manifest(
            "missing-http-mfr",
            "PackagedService",
            "1.0.0",
            "wasm32-wasip1",
        ))),
        mfr_signature: None,
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    let response = post_register(app, request, None).await;

    match response.result.expect("result") {
        register_response::Result::Error(err) => {
            assert_eq!(err.code, 403);
            assert!(err.message.contains("manufacturer not verified"));
        }
        register_response::Result::Success(_) => {
            panic!("package /register without MFR identity should still be rejected")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_linked_registration_with_verified_realm_secret_succeeds() {
    let env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let response = issuer
        .issue_credential_with_realm_secret_check(
            &linked_register_request(),
            Some(RealmSecretCheck::ValidCurrent),
        )
        .await
        .expect("issue linked credential");

    match response.result.expect("result") {
        register_response::Result::Success(ok) => {
            assert_eq!(ok.actr_id.r#type.name, "source-workload");
            assert_eq!(ok.actr_id.realm.realm_id, 1001);
        }
        register_response::Result::Error(err) => {
            panic!("linked registration with verified realm secret should succeed: {err:?}")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_linked_registration_without_verified_realm_secret_is_rejected() {
    let env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let response = issuer
        .issue_credential_with_realm_secret_check(
            &linked_register_request(),
            Some(RealmSecretCheck::NotConfigured),
        )
        .await
        .expect("issue linked credential");

    match response.result.expect("result") {
        register_response::Result::Error(err) => {
            assert_eq!(err.code, 403);
            assert!(err.message.contains("manufacturer not verified"));
        }
        register_response::Result::Success(_) => {
            panic!("linked registration without verified realm secret should be rejected")
        }
    }
}

#[tokio::test]
#[serial]
async fn test_package_registration_without_mfr_identity_is_still_rejected() {
    let env = setup_test_environment().await;

    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "missing-mfr-for-package-auth-test".to_string(),
            name: "package-workload".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(test_registry_manifest(
            "missing-mfr-for-package-auth-test",
            "package-workload",
            "1.0.0",
            "wasm32-wasip1",
        ))),
        mfr_signature: None,
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };

    let response = issuer
        .issue_credential(&request)
        .await
        .expect("issue package credential");

    match response.result.expect("result") {
        register_response::Result::Error(err) => {
            assert_eq!(err.code, 403);
            assert!(err.message.contains("manufacturer not verified"));
        }
        register_response::Result::Success(_) => {
            panic!("package registration without MFR identity should still be rejected")
        }
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

    // Seed database with MFR and package data so verify_mfr_identity path-1 passes
    let manifest_raw = seed_active_package("acme", "test-device", "1.0.0", "wasm32-wasip1").await;

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
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_raw.clone())),
        mfr_signature: None,
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
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
                name: "test-device".to_string(),
                version: "1.0.0".to_string(),
            },
            realm: Realm { realm_id: 1001 },
            service_spec: None,
            acl: None,
            service: None,
            ws_address: None,
            manifest_raw: Some(prost::bytes::Bytes::from(manifest_raw.clone())),
            mfr_signature: None,
            psk_token: None,
            target: Some("wasm32-wasip1".to_string()),
            auth_mode: Some(RegisterAuthMode::Package as i32),
            runner_signature: None,
            runner_signed_at: None,
            runner_nonce: None,
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
        manifest_raw: None,
        mfr_signature: None,
        psk_token: None,
        target: None,
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
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

// ═══════════════════════════════════════════════════════════════════════════
// Path 2 tests: verify_mfr_identity with manifest_raw + mfr_signature
// ═══════════════════════════════════════════════════════════════════════════

/// Helper: build a signed manifest TOML string and its Ed25519 signature.
fn build_signed_manifest(
    signing_key: &ed25519_dalek::SigningKey,
    manufacturer: &str,
    name: &str,
    version: &str,
    target: &str,
) -> (Vec<u8>, Vec<u8>) {
    use ed25519_dalek::Signer;

    let key_id = actrix_mfr::crypto::compute_key_id(&signing_key.verifying_key().to_bytes());
    let manifest = format!(
        r#"manufacturer = "{manufacturer}"
name = "{name}"
version = "{version}"
signing_key_id = "{key_id}"

[binary]
path = "bin/actor.wasm"
target = "{target}"
hash = "0000000000000000000000000000000000000000000000000000000000000000"
"#,
    );
    let manifest_bytes = manifest.into_bytes();
    let signature = signing_key.sign(&manifest_bytes);
    (manifest_bytes, signature.to_bytes().to_vec())
}

fn test_unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn sign_runner_request_at(
    request: &mut RegisterRequest,
    signing_key: &ed25519_dalek::SigningKey,
    manifest_bytes: &[u8],
    signed_at: u64,
    nonce: Vec<u8>,
) {
    use ed25519_dalek::Signer;
    use sha2::{Digest, Sha256};

    let target = request
        .target
        .as_deref()
        .expect("test request should carry target");
    let digest = Sha256::digest(manifest_bytes);
    let manifest_sha256_hex = actr_protocol::lower_hex(&digest);
    let payload =
        actr_protocol::build_runner_register_payload(actr_protocol::RunnerRegisterPayload {
            realm_id: request.realm.realm_id,
            actr_type: &request.actr_type,
            target,
            manifest_sha256_hex: &manifest_sha256_hex,
            runner_signed_at: signed_at,
            runner_nonce: &nonce,
        });
    let signature = signing_key.sign(payload.as_bytes());

    request.runner_signature = Some(prost::bytes::Bytes::from(signature.to_bytes().to_vec()));
    request.runner_signed_at = Some(signed_at);
    request.runner_nonce = Some(prost::bytes::Bytes::from(nonce));
}

fn sign_runner_request(
    request: &mut RegisterRequest,
    signing_key: &ed25519_dalek::SigningKey,
    manifest_bytes: &[u8],
) {
    sign_runner_request_at(
        request,
        signing_key,
        manifest_bytes,
        test_unix_now_secs(),
        vec![0x42; 32],
    );
}

/// Helper: seed an MFR with a specific keypair and return (mfr_id, key_id, public_key_b64).
async fn seed_mfr_with_key(
    pool: &sqlx::SqlitePool,
    mfr_name: &str,
    signing_key: &ed25519_dalek::SigningKey,
) -> (i64, String) {
    let pub_b64 = base64::prelude::BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let key_id = actrix_mfr::crypto::compute_key_id(&signing_key.verifying_key().to_bytes());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let expires_at = now + 86400 * 365;

    sqlx::query(
        "INSERT OR REPLACE INTO mfr (name, public_key, key_id, contact, status, created_at, verified_at, key_expires_at) \
         VALUES (?, ?, ?, 'test@example.com', 'active', ?, ?, ?)"
    )
    .bind(mfr_name)
    .bind(&pub_b64)
    .bind(&key_id)
    .bind(now)
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("seed mfr");

    let mfr_id: i64 = sqlx::query_scalar("SELECT id FROM mfr WHERE name = ?")
        .bind(mfr_name)
        .fetch_one(pool)
        .await
        .expect("get mfr id");

    (mfr_id, key_id)
}

/// Helper: archive a key into mfr_key_history with given status.
async fn archive_key_to_history(
    pool: &sqlx::SqlitePool,
    mfr_id: i64,
    key_id: &str,
    public_key_b64: &str,
    status: &str,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    sqlx::query(
        "INSERT INTO mfr_key_history (mfr_id, key_id, public_key, status, created_at, retired_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(mfr_id)
    .bind(key_id)
    .bind(public_key_b64)
    .bind(status)
    .bind(now - 1000)
    .bind(now)
    .execute(pool)
    .await
    .expect("archive key to history");
}

/// Path 2: missing runner_signature is rejected for unpublished packages.
#[tokio::test]
#[serial]
async fn test_path2_missing_runner_signature_rejected() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let env = setup_test_environment().await;
    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let pool = platform::storage::db::get_database().get_pool();
    seed_realm(1001, "test", None).await;

    let key = SigningKey::generate(&mut OsRng);
    let (_mfr_id, _key_id) = seed_mfr_with_key(pool, "missrunner", &key).await;
    let (manifest_bytes, sig_bytes) = build_signed_manifest(
        &key,
        "missrunner",
        "MissingRunnerService",
        "1.0.0",
        "wasm32-wasip1",
    );

    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "missrunner".to_string(),
            name: "MissingRunnerService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_bytes)),
        mfr_signature: Some(prost::bytes::Bytes::from(sig_bytes)),
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };

    let response = issuer.issue_credential(&request).await.expect("issue");
    match response.result.expect("result") {
        register_response::Result::Error(_) => {}
        register_response::Result::Success(_) => {
            panic!("Path 2 without runner_signature should be rejected")
        }
    }
}

/// Path 2: replaying the same runner_nonce is rejected after the first success.
#[tokio::test]
#[serial]
async fn test_path2_runner_nonce_replay_rejected() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let env = setup_test_environment().await;
    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let pool = platform::storage::db::get_database().get_pool();
    seed_realm(1001, "test", None).await;

    let key = SigningKey::generate(&mut OsRng);
    let (_mfr_id, _key_id) = seed_mfr_with_key(pool, "replaymfr", &key).await;
    let (manifest_bytes, sig_bytes) =
        build_signed_manifest(&key, "replaymfr", "ReplayService", "1.0.0", "wasm32-wasip1");

    let mut request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "replaymfr".to_string(),
            name: "ReplayService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_bytes.clone())),
        mfr_signature: Some(prost::bytes::Bytes::from(sig_bytes)),
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    sign_runner_request_at(
        &mut request,
        &key,
        &manifest_bytes,
        test_unix_now_secs(),
        vec![0x99; 32],
    );

    let first = issuer
        .issue_credential(&request)
        .await
        .expect("first issue");
    match first.result.expect("first result") {
        register_response::Result::Success(_) => {}
        register_response::Result::Error(err) => {
            panic!("first Path 2 registration should succeed, got error: {err:?}")
        }
    }

    let second = issuer
        .issue_credential(&request)
        .await
        .expect("second issue");
    match second.result.expect("second result") {
        register_response::Result::Error(_) => {}
        register_response::Result::Success(_) => {
            panic!("replayed runner_nonce should be rejected")
        }
    }
}

/// Path 2: historical (retired) key passes signature verification.
#[tokio::test]
#[serial]
async fn test_path2_historical_retired_key_passes() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let env = setup_test_environment().await;
    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let pool = platform::storage::db::get_database().get_pool();

    // Key A (old) and Key B (current)
    let key_a = SigningKey::generate(&mut OsRng);
    let key_b = SigningKey::generate(&mut OsRng);

    let key_a_pub_b64 = base64::prelude::BASE64_STANDARD.encode(key_a.verifying_key().to_bytes());
    let key_a_id = actrix_mfr::crypto::compute_key_id(&key_a.verifying_key().to_bytes());

    // Seed MFR with Key B as current
    let (mfr_id, _key_b_id) = seed_mfr_with_key(pool, "histmfr", &key_b).await;

    // Archive Key A as retired
    archive_key_to_history(pool, mfr_id, &key_a_id, &key_a_pub_b64, "retired").await;

    // Seed realm
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    sqlx::query("INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES (1001, 'test', 'Active', 1, ?, '')")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed realm");

    // Build manifest signed by Key A (the retired key)
    let (manifest_bytes, sig_bytes) =
        build_signed_manifest(&key_a, "histmfr", "HistService", "1.0.0", "wasm32-wasip1");

    let mut request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "histmfr".to_string(),
            name: "HistService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_bytes.clone())),
        mfr_signature: Some(prost::bytes::Bytes::from(sig_bytes)),
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    sign_runner_request(&mut request, &key_a, &manifest_bytes);

    let response = issuer.issue_credential(&request).await.expect("issue");
    match response.result.expect("result") {
        register_response::Result::Success(_) => {} // expected
        register_response::Result::Error(err) => {
            panic!("Path 2 with retired historical key should succeed, got error: {err:?}")
        }
    }
}

/// Path 2: revoked key is rejected.
#[tokio::test]
#[serial]
async fn test_path2_revoked_key_rejected() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let env = setup_test_environment().await;
    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let pool = platform::storage::db::get_database().get_pool();

    // Seed realm
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    sqlx::query("INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES (1001, 'test', 'Active', 1, ?, '')")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed realm");

    let key_revoked = SigningKey::generate(&mut OsRng);
    let key_current = SigningKey::generate(&mut OsRng);

    let revoked_pub_b64 =
        base64::prelude::BASE64_STANDARD.encode(key_revoked.verifying_key().to_bytes());
    let revoked_key_id =
        actrix_mfr::crypto::compute_key_id(&key_revoked.verifying_key().to_bytes());

    // Seed MFR with current key
    let (mfr_id, _) = seed_mfr_with_key(pool, "revmfr", &key_current).await;

    // Archive revoked key
    archive_key_to_history(pool, mfr_id, &revoked_key_id, &revoked_pub_b64, "revoked").await;

    // Build manifest signed by the revoked key
    let (manifest_bytes, sig_bytes) = build_signed_manifest(
        &key_revoked,
        "revmfr",
        "RevService",
        "1.0.0",
        "wasm32-wasip1",
    );

    let mut request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "revmfr".to_string(),
            name: "RevService".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_bytes.clone())),
        mfr_signature: Some(prost::bytes::Bytes::from(sig_bytes)),
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    sign_runner_request(&mut request, &key_revoked, &manifest_bytes);

    let response = issuer.issue_credential(&request).await.expect("issue");
    match response.result.expect("result") {
        register_response::Result::Error(_) => {} // expected: revoked key rejected
        register_response::Result::Success(_) => {
            panic!("Path 2 with revoked key should be rejected, but got success")
        }
    }
}

/// Path 2: manifest identity mismatch (actr_type spoofing) is rejected.
#[tokio::test]
#[serial]
async fn test_path2_identity_mismatch_rejected() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let env = setup_test_environment().await;
    let signer_client = create_signer_client(&env.signer_config, &env.shared_key)
        .await
        .expect("signer client");
    let issuer = AIdIssuer::new(
        signer_client,
        default_issuer_config(&env.issuer_temp_dir),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("issuer");

    let pool = platform::storage::db::get_database().get_pool();

    // Seed realm (shared global DB may not have it if run in isolation)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    sqlx::query("INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES (1001, 'test', 'Active', 1, ?, '')")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed realm");

    let key = SigningKey::generate(&mut OsRng);
    let (_mfr_id, _key_id) = seed_mfr_with_key(pool, "spoofmfr", &key).await;

    // Build manifest for ServiceA
    let (manifest_bytes, sig_bytes) =
        build_signed_manifest(&key, "spoofmfr", "ServiceA", "1.0.0", "wasm32-wasip1");

    // But register as ServiceB — identity mismatch!
    let mut request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "spoofmfr".to_string(),
            name: "ServiceB".to_string(), // ← does not match manifest
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
        service: None,
        ws_address: None,
        manifest_raw: Some(prost::bytes::Bytes::from(manifest_bytes.clone())),
        mfr_signature: Some(prost::bytes::Bytes::from(sig_bytes)),
        psk_token: None,
        target: Some("wasm32-wasip1".to_string()),
        auth_mode: Some(RegisterAuthMode::Package as i32),
        runner_signature: None,
        runner_signed_at: None,
        runner_nonce: None,
    };
    sign_runner_request(&mut request, &key, &manifest_bytes);

    let response = issuer.issue_credential(&request).await.expect("issue");
    match response.result.expect("result") {
        register_response::Result::Error(_) => {} // expected: identity mismatch rejected
        register_response::Result::Success(_) => {
            panic!("Path 2 with identity mismatch should be rejected, but got success")
        }
    }
}
