use super::*;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
#[cfg(feature = "dynclib-engine")]
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

fn dev_config(dir: &TempDir) -> HyperConfig {
    let signing_key = SigningKey::generate(&mut OsRng);
    let pubkey = signing_key.verifying_key().to_bytes();
    HyperConfig::new(
        dir.path(),
        Arc::new(crate::verify::StaticTrust::new(pubkey).unwrap()),
    )
}

#[tokio::test]
async fn init_creates_data_dir_and_instance_id() {
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("subdir/nested");
    let signing_key = SigningKey::generate(&mut OsRng);
    let config = HyperConfig::new(
        &sub,
        Arc::new(crate::verify::StaticTrust::new(signing_key.verifying_key().to_bytes()).unwrap()),
    );

    let hyper = Hyper::new(config).await.unwrap();
    assert!(sub.exists());
    assert!(!hyper.instance_id().is_empty());
}

#[tokio::test]
async fn instance_id_is_stable_across_reinit() {
    let dir = TempDir::new().unwrap();
    let config1 = dev_config(&dir);
    let hyper1 = Hyper::new(config1).await.unwrap();
    let id1 = hyper1.instance_id().to_string();

    let config2 = dev_config(&dir);
    let hyper2 = Hyper::new(config2).await.unwrap();
    let id2 = hyper2.instance_id().to_string();

    assert_eq!(id1, id2, "instance_id should remain stable across restarts");
}

#[tokio::test]
async fn verify_package_rejects_non_wasm() {
    let dir = TempDir::new().unwrap();
    let hyper = Hyper::new(dev_config(&dir)).await.unwrap();
    let result = hyper
        .verify_package(&WorkloadPackage::new(b"not a wasm file".to_vec()))
        .await;
    assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
}

#[tokio::test]
async fn verify_package_rejects_non_actr_format() {
    let dir = TempDir::new().unwrap();
    let hyper = Hyper::new(dev_config(&dir)).await.unwrap();

    // Non-.actr bytes should return InvalidManifest
    let result = hyper
        .verify_package(&WorkloadPackage::new(b"\0asm\x01\x00\x00\x00".to_vec()))
        .await;
    assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
}

// ─── ActorStore helpers ────────────────────────────────────────────────

#[allow(dead_code)]
async fn open_test_store(dir: &TempDir) -> ActorStore {
    let db_path = dir.path().join("test.db");
    ActorStore::open(&db_path).await.unwrap()
}

// ─── AIS integration tests (mockito mock server) ────────────────────────

/// Helper: build a [`VerifiedPackage`] for tests.
///
/// Uses the canonical `actr_pack::PackageManifest` shape wrapped with empty
/// manifest_raw / sig_raw placeholders — bootstrap tests don't touch AIS's
/// re-verification path, so those bytes are not inspected.
fn fake_manifest() -> VerifiedPackage {
    VerifiedPackage {
        manifest: actr_pack::PackageManifest {
            manufacturer: "test-mfr".to_string(),
            name: "TestActor".to_string(),
            version: "0.1.0".to_string(),
            binary: actr_pack::BinaryEntry {
                path: "bin/actor.wasm".to_string(),
                target: "wasm32-wasip1".to_string(),
                hash: "0".repeat(64),
                size: None,
                kind: None,
            },
            signature_algorithm: "ed25519".to_string(),
            signing_key_id: None,
            resources: vec![],
            proto_files: vec![],
            lock_file: None,
            metadata: actr_pack::ManifestMetadata::default(),
        },
        manifest_raw: vec![],
        sig_raw: vec![0u8; 64],
    }
}

/// Helper: build valid RegisterResponse protobuf bytes with credential data.
fn fake_register_response_bytes() -> Vec<u8> {
    use actr_protocol::{
        AIdCredential, ActrId, ActrType, IdentityClaims, Realm, RegisterResponse, TurnCredential,
        register_response,
    };

    let claims = IdentityClaims {
        realm_id: 1,
        actor_id: "test-actor-id".to_string(),
        expires_at: u64::MAX,
    };
    let claims_bytes = claims.encode_to_vec();

    let credential = AIdCredential {
        key_id: 1,
        claims: claims_bytes.into(),
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
        username: "user".to_string(),
        password: "pass".to_string(),
        expires_at: u64::MAX,
    };

    let ok = register_response::RegisterOk {
        actr_id,
        credential,
        turn_credential: turn,
        credential_expires_at: None,
        signaling_heartbeat_interval_secs: 30,
        signing_pubkey: vec![0u8; 32].into(),
        signing_key_id: 1,
        renewal_token: None,
        renewal_token_expires_at: None,
    };

    RegisterResponse {
        result: Some(register_response::Result::Success(ok)),
    }
    .encode_to_vec()
}

fn test_service_spec() -> Option<ServiceSpec> {
    Some(ServiceSpec {
        name: "EchoService".to_string(),
        description: Some("test service".to_string()),
        fingerprint: "fp-123".to_string(),
        protobufs: vec![],
        published_at: None,
        tags: vec!["latest".to_string()],
    })
}

fn test_acl() -> Option<Acl> {
    Some(Acl { rules: vec![] })
}

fn linked_runtime_config(dir: &TempDir) -> actr_config::RuntimeConfig {
    actr_config::RuntimeConfig {
        package: actr_config::PackageInfo {
            name: "LinkedActor".to_string(),
            actr_type: actr_protocol::ActrType {
                manufacturer: "test-mfr".to_string(),
                name: "LinkedActor".to_string(),
                version: "0.1.0".to_string(),
            },
            description: None,
            authors: vec![],
            license: None,
        },
        signaling_url: url::Url::parse("ws://localhost:8081/signaling/ws").unwrap(),
        realm: Realm { realm_id: 7 },
        ais_endpoint: "http://localhost:8081/ais".to_string(),
        realm_secret: Some("test-realm-secret".to_string()),
        visible_in_discovery: true,
        acl: test_acl(),
        mailbox_path: None,
        scripts: std::collections::HashMap::new(),
        webrtc: actr_config::WebRtcConfig::default(),
        websocket_listen_port: Some(9100),
        websocket_advertised_host: Some("127.0.0.1".to_string()),
        observability: actr_config::ObservabilityConfig {
            filter_level: "info".to_string(),
            tracing_enabled: false,
            tracing_endpoint: "http://localhost:4317".to_string(),
            tracing_service_name: "linked-test".to_string(),
        },
        config_dir: dir.path().to_path_buf(),
        trust: vec![],
        package_path: None,
        web: None,
    }
}

#[test]
fn linked_register_request_uses_linked_auth_mode() {
    let dir = TempDir::new().unwrap();
    let req = build_linked_register_request(&linked_runtime_config(&dir), test_service_spec());

    assert_eq!(req.auth_mode, Some(RegisterAuthMode::Linked as i32));
    assert_eq!(req.manifest_raw, None);
    assert_eq!(req.mfr_signature, None);
    assert_eq!(req.ws_address.as_deref(), Some("ws://127.0.0.1:9100"));
}

#[tokio::test]
async fn with_actor_type_overrides_pending_runtime_metadata() {
    let dir = TempDir::new().unwrap();
    let hyper = Hyper::new(dev_config(&dir)).await.unwrap();
    let node = Node::from_hyper(hyper, linked_runtime_config(&dir)).with_actor_type(
        actr_protocol::ActrType {
            manufacturer: "acme".into(),
            name: "UnifiedActor".into(),
            version: "1.0.0".into(),
        },
    );

    let actr_type = node.runtime_config().actr_type();
    assert_eq!(actr_type.manufacturer, "acme");
    assert_eq!(actr_type.name, "UnifiedActor");
    assert_eq!(actr_type.version, "1.0.0");
}

#[test]
fn compatible_native_target_matches_current_host() {
    // Current host should always match itself.
    let current = format!(
        "{}-unknown-{}",
        std::env::consts::ARCH,
        if std::env::consts::OS == "macos" {
            "darwin"
        } else {
            std::env::consts::OS
        }
    );
    assert!(
        is_compatible_native_target(&current),
        "current host target `{current}` should be compatible"
    );
}

#[test]
fn compatible_native_target_rejects_cross_platform() {
    // A target for a different arch/os should be rejected.
    assert!(!is_compatible_native_target("riscv64gc-unknown-linux-gnu"));
    assert!(!is_compatible_native_target("s390x-unknown-linux-gnu"));
}

#[test]
fn compatible_native_target_rejects_short_triples() {
    assert!(!is_compatible_native_target("invalid-target"));
    assert!(!is_compatible_native_target("single"));
    assert!(!is_compatible_native_target(""));
}

#[cfg(feature = "dynclib-engine")]
fn fake_dynclib_manifest() -> PackageManifest {
    let target = format!(
        "{}-unknown-{}",
        std::env::consts::ARCH,
        if std::env::consts::OS == "macos" {
            "darwin"
        } else {
            std::env::consts::OS
        }
    );
    PackageManifest {
        manufacturer: "test-mfr".to_string(),
        name: "DynActor".to_string(),
        version: "1.0.0".to_string(),
        binary: actr_pack::BinaryEntry {
            path: format!("bin/actor{}", dynclib_tempfile_suffix()),
            target,
            hash: String::new(),
            size: None,
            kind: None,
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: None,
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        metadata: actr_pack::ManifestMetadata::default(),
    }
}

#[cfg(feature = "dynclib-engine")]
fn fake_dynclib_package_bytes(binary_bytes: &[u8]) -> (Vec<u8>, PackageManifest) {
    let manifest = fake_dynclib_manifest();
    let signing_key = SigningKey::generate(&mut OsRng);
    let package_bytes = actr_pack::pack(&actr_pack::PackOptions {
        manifest: manifest.clone(),
        binary_bytes: binary_bytes.to_vec(),
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        signing_key,
    })
    .unwrap();
    // `pack()` updates the embedded manifest's binary hash; re-parse so
    // the returned manifest agrees with what's actually in the archive.
    let packed_manifest = actr_pack::read_manifest(&package_bytes).unwrap();
    (package_bytes, packed_manifest)
}

#[cfg(feature = "dynclib-engine")]
#[test]
fn dynclib_cache_path_uses_hash_and_platform_suffix() {
    let dir = TempDir::new().unwrap();
    let path = dynclib_cache_path(dir.path(), &[0xAB; 32]);

    assert_eq!(path.parent().unwrap(), dynclib_cache_dir(dir.path()));
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        format!("{}{}", hex::encode([0xAB; 32]), dynclib_tempfile_suffix())
    );
}

#[cfg(feature = "dynclib-engine")]
#[test]
fn ensure_dynclib_cache_path_preserves_existing_file() {
    let dir = TempDir::new().unwrap();
    let initial_binary_bytes = b"initial dylib bytes";
    let (initial_package_bytes, manifest) = fake_dynclib_package_bytes(initial_binary_bytes);
    let cache_path =
        ensure_dynclib_cache_path(dir.path(), &initial_package_bytes, &manifest).unwrap();

    // Same initial binary -> same manifest.binary.hash -> same cache path;
    // a second call with a different binary under that hash cannot land
    // here, so re-run with the identical binary to assert idempotence.
    let second_path =
        ensure_dynclib_cache_path(dir.path(), &initial_package_bytes, &manifest).unwrap();

    assert_eq!(cache_path, second_path);
    assert_eq!(std::fs::read(&cache_path).unwrap(), initial_binary_bytes);
}

#[cfg(feature = "dynclib-engine")]
#[test]
fn ensure_dynclib_cache_path_handles_concurrent_creation() {
    let dir = TempDir::new().unwrap();
    let binary_bytes = b"shared dylib bytes".to_vec();
    let (package_bytes, manifest) = fake_dynclib_package_bytes(&binary_bytes);
    let package_bytes = Arc::new(package_bytes);
    let binary_bytes = Arc::new(binary_bytes);
    let data_dir = Arc::new(dir.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(3));

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let data_dir = Arc::clone(&data_dir);
            let manifest = manifest.clone();
            let package_bytes = Arc::clone(&package_bytes);
            std::thread::spawn(move || {
                barrier.wait();
                ensure_dynclib_cache_path(&data_dir, &package_bytes, &manifest)
            })
        })
        .collect();

    barrier.wait();

    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap().unwrap())
        .collect();

    assert_eq!(results[0], results[1]);
    assert_eq!(
        std::fs::read(&results[0]).unwrap(),
        binary_bytes.as_ref().as_slice()
    );
}

/// Package bootstrap always authenticates with the MFR manifest and does
/// not read or write legacy PSK keys.
#[tokio::test]
async fn bootstrap_package_uses_manifest_auth() {
    let response_body = fake_register_response_bytes();

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(response_body)
        .expect(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let config = dev_config(&dir);
    let hyper = Hyper::new(config).await.unwrap();

    let manifest = fake_manifest();
    let result = hyper
        .bootstrap_credential(&manifest, &server.url(), 1, test_service_spec(), test_acl())
        .await;

    mock.assert_async().await;
    assert!(
        result.is_ok(),
        "Package registration should succeed, got: {:?}",
        result.err()
    );

    // Verify no legacy PSK keys are written to ActorStore.
    let storage_path = hyper.resolve_storage_path(&manifest.manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();
    assert!(
        store.kv_get("hyper:psk:token").await.unwrap().is_none(),
        "PSK token must not be stored after PSK removal"
    );
}

/// AIS errors should propagate as HyperError::AisBootstrapFailed.
#[tokio::test]
async fn bootstrap_ais_error_propagates() {
    use actr_protocol::{ErrorResponse, RegisterResponse, register_response};

    let error_resp = RegisterResponse {
        result: Some(register_response::Result::Error(ErrorResponse {
            code: 403,
            message: "manufacturer not trusted".to_string(),
        })),
    }
    .encode_to_vec();

    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(error_resp)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let config = dev_config(&dir);
    let hyper = Hyper::new(config).await.unwrap();

    let manifest = fake_manifest();
    let result = hyper
        .bootstrap_credential(&manifest, &server.url(), 1, test_service_spec(), test_acl())
        .await;

    assert!(
        matches!(result, Err(HyperError::AisBootstrapFailed(_))),
        "AIS errors should propagate as AisBootstrapFailed, got: {:?}",
        result
    );
}

// ── pure helper functions ───────────────────────────────────────────────

use actr_protocol::{ActrType, Realm, RegisterAuthMode};

fn runtime_config_with_ws(port: Option<u16>, host: Option<&str>) -> actr_config::RuntimeConfig {
    actr_config::RuntimeConfig {
        package: actr_config::PackageInfo {
            name: "linked".into(),
            actr_type: ActrType {
                manufacturer: "mfr".into(),
                name: "Linked".into(),
                version: "1.0.0".into(),
            },
            description: None,
            authors: vec![],
            license: None,
        },
        signaling_url: url::Url::parse("ws://127.0.0.1:9/signaling/ws").unwrap(),
        realm: Realm { realm_id: 1 },
        ais_endpoint: "http://127.0.0.1:9/ais".into(),
        realm_secret: None,
        visible_in_discovery: true,
        acl: None,
        mailbox_path: None,
        scripts: std::collections::HashMap::new(),
        webrtc: actr_config::WebRtcConfig::default(),
        websocket_listen_port: port,
        websocket_advertised_host: host.map(|h| h.to_string()),
        observability: actr_config::ObservabilityConfig {
            filter_level: "info".into(),
            tracing_enabled: false,
            tracing_endpoint: String::new(),
            tracing_service_name: "test".into(),
        },
        config_dir: std::path::PathBuf::from("."),
        trust: vec![],
        package_path: None,
        web: None,
    }
}

#[test]
fn build_linked_register_request_includes_ws_address_when_port_set() {
    let cfg = runtime_config_with_ws(Some(8090), Some("example.com"));
    let req = build_linked_register_request(&cfg, None);
    assert_eq!(req.actr_type, cfg.actr_type().clone());
    assert_eq!(req.realm, cfg.realm);
    assert_eq!(req.auth_mode, Some(RegisterAuthMode::Linked as i32));
    assert_eq!(req.ws_address.as_deref(), Some("ws://example.com:8090"));
}

#[test]
fn build_linked_register_request_omits_ws_address_without_port() {
    let cfg = runtime_config_with_ws(None, None);
    let req = build_linked_register_request(&cfg, None);
    assert!(req.ws_address.is_none());
    // Default host fallback only applies when port is Some.
}

#[test]
fn build_linked_register_request_defaults_host_to_localhost() {
    let cfg = runtime_config_with_ws(Some(7000), None);
    let req = build_linked_register_request(&cfg, None);
    assert_eq!(req.ws_address.as_deref(), Some("ws://127.0.0.1:7000"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn is_compatible_native_target_validates_host_triple() {
    // The current host triple must be accepted.
    let host = format!(
        "{}-unknown-{}",
        std::env::consts::ARCH,
        if std::env::consts::OS == "macos" {
            "darwin"
        } else {
            std::env::consts::OS
        }
    );
    assert!(is_compatible_native_target(&host));

    // Bogus / too-short triples are rejected.
    assert!(!is_compatible_native_target("not-a-triple"));
    assert!(!is_compatible_native_target("only-two"));
    assert!(!is_compatible_native_target(""));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn detect_binary_kind_rejects_unsupported_target() {
    let manifest = actr_pack::PackageManifest {
        manufacturer: "m".into(),
        name: "n".into(),
        version: "1.0.0".into(),
        binary: actr_pack::BinaryEntry {
            path: "bin/x".into(),
            target: "bogus-arch-unknown-os".into(),
            hash: String::new(),
            size: None,
            kind: None,
        },
        signature_algorithm: "ed25519".into(),
        signing_key_id: None,
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        metadata: actr_pack::ManifestMetadata::default(),
    };
    let err = detect_binary_kind(&manifest).unwrap_err();
    assert!(matches!(err, HyperError::InvalidManifest(_)));
}
