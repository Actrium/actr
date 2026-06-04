#![cfg(any(feature = "wasm-engine", feature = "dynclib-engine"))]

use std::collections::HashMap;
#[cfg(feature = "dynclib-engine")]
use std::path::{Path, PathBuf};
use std::sync::Arc;

use actr_hyper::test_support::{TestSignalingServer, attached_node_has_hook_observer};
use actr_hyper::{Hyper, HyperConfig, Node, StaticTrust, WorkloadPackage};
use actr_protocol::{ActrType, Realm};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use tempfile::TempDir;

#[cfg(feature = "wasm-engine")]
#[path = "wasm_actor_fixture.rs"]
mod wasm_actor_fixture;

fn dev_config_with_key(dir: &TempDir, verifying_key: &ed25519_dalek::VerifyingKey) -> HyperConfig {
    HyperConfig::new(
        dir.path(),
        Arc::new(StaticTrust::new(verifying_key.to_bytes()).unwrap()),
    )
}

fn test_runtime_config(
    dir: &TempDir,
    server: &TestSignalingServer,
    name: &str,
) -> actr_config::RuntimeConfig {
    let actr_type = ActrType {
        manufacturer: "test-mfr".to_string(),
        name: name.to_string(),
        version: "1.0.0".to_string(),
    };

    actr_config::RuntimeConfig {
        package: actr_config::PackageInfo {
            name: name.to_string(),
            actr_type,
            description: None,
            authors: vec![],
            license: None,
        },
        signaling_url: url::Url::parse(&server.url()).unwrap(),
        realm: Realm { realm_id: 7 },
        ais_endpoint: ais_endpoint(server),
        realm_secret: None,
        visible_in_discovery: true,
        acl: None,
        mailbox_path: None,
        scripts: HashMap::new(),
        webrtc: actr_config::WebRtcConfig::default(),
        websocket_listen_port: None,
        websocket_advertised_host: None,
        observability: actr_config::ObservabilityConfig {
            filter_level: "info".to_string(),
            tracing_enabled: false,
            tracing_endpoint: "http://localhost:4317".to_string(),
            tracing_service_name: "package-lifecycle-test".to_string(),
        },
        config_dir: dir.path().to_path_buf(),
        trust: vec![],
        package_path: None,
        web: None,
    }
}

fn ais_endpoint(server: &TestSignalingServer) -> String {
    format!("http://127.0.0.1:{}/ais", server.port())
}

fn build_package(
    binary: &[u8],
    name: &str,
    target: &str,
    path: String,
    kind: Option<actr_pack::BinaryKind>,
    signing_key: &SigningKey,
) -> WorkloadPackage {
    let manifest = actr_pack::PackageManifest {
        manufacturer: "test-mfr".to_string(),
        name: name.to_string(),
        version: "1.0.0".to_string(),
        binary: actr_pack::BinaryEntry {
            path,
            target: target.to_string(),
            hash: String::new(),
            size: None,
            kind,
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: None,
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        metadata: actr_pack::ManifestMetadata::default(),
    };

    WorkloadPackage::new(
        actr_pack::pack(&actr_pack::PackOptions {
            manifest,
            binary_bytes: binary.to_vec(),
            resources: vec![],
            proto_files: vec![],
            lock_file: None,
            signing_key: signing_key.clone(),
        })
        .unwrap(),
    )
}

async fn assert_package_on_start_failure(
    package: WorkloadPackage,
    verifying_key: ed25519_dalek::VerifyingKey,
    actor_name: &str,
) {
    let mut server = TestSignalingServer::start()
        .await
        .expect("mock actrix server should start");
    let dir = TempDir::new().unwrap();
    let hyper = Hyper::new(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();
    let config = test_runtime_config(&dir, &server, actor_name);
    let ais_endpoint = ais_endpoint(&server);

    let attached = Node::from_hyper(hyper, config)
        .attach(&package)
        .await
        .expect("package should attach");
    assert!(
        attached_node_has_hook_observer(&attached),
        "package attach should install hook observer"
    );
    let registered = attached
        .register(&ais_endpoint)
        .await
        .expect("package should register with mock AIS");

    let err = match registered.start().await {
        Ok(_) => panic!("start should abort when package on_start returns Err"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("fixture lifecycle on_start failed"),
        "start error should include fixture on_start failure, got: {err}"
    );

    server.shutdown().await;
}

#[cfg(feature = "wasm-engine")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_wasm_package_invokes_on_start_and_aborts_on_error() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let package = build_package(
        wasm_actor_fixture::WASM_ACTOR_FIXTURE,
        "LifecycleWasm",
        "wasm32-wasip2",
        "bin/actor.wasm".to_string(),
        Some(actr_pack::BinaryKind::Component),
        &signing_key,
    );

    assert_package_on_start_failure(package, signing_key.verifying_key(), "LifecycleWasm").await;
}

#[cfg(feature = "dynclib-engine")]
fn fixture_so_path() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/dynclib_actor_fixture");

    let status = std::process::Command::new("cargo")
        .args(["build"])
        .current_dir(&fixture_dir)
        .status()
        .expect("failed to build dynclib fixture");
    assert!(status.success(), "dynclib fixture build failed");

    let target_dir = manifest_dir.join("../../target/core-hyper-tests-dynclib-actor-fixture/debug");
    if cfg!(target_os = "linux") {
        target_dir.join("libdynclib_actor_fixture.so")
    } else if cfg!(target_os = "macos") {
        target_dir.join("libdynclib_actor_fixture.dylib")
    } else {
        target_dir.join("dynclib_actor_fixture.dll")
    }
}

#[cfg(feature = "dynclib-engine")]
fn current_native_target() -> String {
    format!(
        "{}-unknown-{}",
        std::env::consts::ARCH,
        if std::env::consts::OS == "macos" {
            "darwin"
        } else {
            std::env::consts::OS
        }
    )
}

#[cfg(feature = "dynclib-engine")]
fn dynclib_suffix() -> &'static str {
    if cfg!(target_os = "linux") {
        ".so"
    } else if cfg!(target_os = "macos") {
        ".dylib"
    } else if cfg!(target_os = "windows") {
        ".dll"
    } else {
        ".dynlib"
    }
}

#[cfg(feature = "dynclib-engine")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_dynclib_package_invokes_on_start_and_aborts_on_error() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let dylib_bytes = std::fs::read(fixture_so_path()).unwrap();
    let package = build_package(
        &dylib_bytes,
        "LifecycleDynClib",
        &current_native_target(),
        format!("bin/actor{}", dynclib_suffix()),
        None,
        &signing_key,
    );

    assert_package_on_start_failure(package, signing_key.verifying_key(), "LifecycleDynClib").await;
}
