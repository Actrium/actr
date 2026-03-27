//! Integration tests: .actr package sign → Hyper::verify_package full flow
//!
//! Scenarios:
//! 1. .actr package: pack → verify → manifest fields match
//! 2. .actr package: tampered binary → hash mismatch
//! 3. .actr package: wrong key → signature verification failed
//! 4. Unsigned bytes → InvalidManifest (unrecognized format)

#[cfg(feature = "wasm-engine")]
use actr_hyper::PackageExecutionBackend;
use actr_hyper::{Hyper, HyperConfig, HyperError, TrustMode, WorkloadPackage};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── Utility functions ───────────────────────────────────────────────────────

fn minimal_wasm() -> Vec<u8> {
    b"\0asm\x01\x00\x00\x00".to_vec()
}

#[cfg(feature = "wasm-engine")]
fn echo_guest_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"
(module
  (memory (export "memory") 2)
  (global $heap (mut i32) (i32.const 4096))
  (func $bump (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))
  (func (export "actr_alloc") (param $n i32) (result i32)
    (call $bump (local.get $n)))
  (func (export "actr_free") (param $p i32) (param $n i32))
  (func (export "asyncify_start_unwind") (param i32))
  (func (export "asyncify_stop_unwind"))
  (func (export "asyncify_start_rewind") (param i32))
  (func (export "asyncify_stop_rewind"))
  (func (export "actr_init") (param $p i32) (param $n i32) (result i32)
    (i32.const 0))
  (func (export "actr_handle")
    (param $req_ptr i32) (param $req_len i32)
    (param $resp_ptr_out i32) (param $resp_len_out i32)
    (result i32)
    (local $resp_ptr i32)
    (local.set $resp_ptr (call $bump (local.get $req_len)))
    (memory.copy
      (local.get $resp_ptr)
      (local.get $req_ptr)
      (local.get $req_len))
    (i32.store (local.get $resp_ptr_out) (local.get $resp_ptr))
    (i32.store (local.get $resp_len_out) (local.get $req_len))
    (i32.const 0))
)
"#,
    )
    .expect("WAT parse failed")
}

/// Build an .actr ZIP package
fn build_actr_package(
    binary: &[u8],
    manufacturer: &str,
    name: &str,
    version: &str,
    signing_key: &SigningKey,
) -> Vec<u8> {
    let manifest = actr_pack::PackageManifest {
        manufacturer: manufacturer.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        binary: actr_pack::BinaryEntry {
            path: "bin/actor.wasm".to_string(),
            target: "wasm32-wasip1".to_string(),
            hash: String::new(),
            size: None,
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: None,
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        metadata: actr_pack::ManifestMetadata::default(),
    };
    let opts = actr_pack::PackOptions {
        manifest,
        binary_bytes: binary.to_vec(),
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        signing_key: signing_key.clone(),
    };
    actr_pack::pack(&opts).unwrap()
}

fn dev_config_with_key(dir: &TempDir, verifying_key: &ed25519_dalek::VerifyingKey) -> HyperConfig {
    HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Development {
        self_signed_pubkey: verifying_key.to_bytes().to_vec(),
    })
}

// ─── .actr package tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn actr_package_roundtrip() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let wasm = minimal_wasm();
    let package = build_actr_package(&wasm, "test-mfr", "MyActor", "1.2.3", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    let manifest = hyper
        .verify_package(&WorkloadPackage::new(package))
        .await
        .unwrap();
    assert_eq!(manifest.manufacturer, "test-mfr");
    assert_eq!(manifest.actr_name, "MyActor");
    assert_eq!(manifest.version, "1.2.3");
}

#[tokio::test]
async fn actr_package_tampered_binary() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let package = build_actr_package(b"original wasm", "mfr", "A", "1.0", &signing_key);

    // Tamper: find "original wasm" in the ZIP and modify it
    let mut tampered = package.clone();
    if let Some(pos) = tampered.windows(13).position(|w| w == b"original wasm") {
        tampered[pos] ^= 0xFF;
    }

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();
    let result = hyper.verify_package(&WorkloadPackage::new(tampered)).await;
    assert!(
        result.is_err(),
        "tampered .actr package should fail verification"
    );
}

#[tokio::test]
async fn actr_package_wrong_key() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let wrong_key = SigningKey::generate(&mut OsRng);
    let wrong_verifying = wrong_key.verifying_key();

    let package = build_actr_package(b"wasm", "mfr", "A", "1.0", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &wrong_verifying))
        .await
        .unwrap();
    let result = hyper.verify_package(&WorkloadPackage::new(package)).await;
    assert!(
        matches!(result, Err(HyperError::SignatureVerificationFailed(_))),
        "wrong key should return SignatureVerificationFailed, got: {result:?}"
    );
}

#[tokio::test]
async fn verify_rejects_unknown_format() {
    let dir = TempDir::new().unwrap();
    let signing_key = SigningKey::generate(&mut OsRng);
    let hyper = Hyper::init(dev_config_with_key(&dir, &signing_key.verifying_key()))
        .await
        .unwrap();

    let result = hyper
        .verify_package(&WorkloadPackage::new(b"this is not a binary".to_vec()))
        .await;
    assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
}

#[cfg(feature = "wasm-engine")]
#[tokio::test]
async fn load_workload_package_selects_wasm_backend() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let package = build_actr_package(
        &echo_guest_wasm(),
        "test-mfr",
        "Echo",
        "1.0.0",
        &signing_key,
    );

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    let loaded = hyper
        .load_workload_package(&WorkloadPackage::new(package))
        .await
        .unwrap();

    assert_eq!(loaded.backend, PackageExecutionBackend::Wasm);
    assert_eq!(loaded.manifest.binary_target, "wasm32-wasip1");
}

#[cfg(feature = "wasm-engine")]
#[tokio::test]
async fn load_workload_package_rejects_second_load_for_same_hyper() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let package = build_actr_package(
        &echo_guest_wasm(),
        "test-mfr",
        "Echo",
        "1.0.0",
        &signing_key,
    );

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    hyper
        .load_workload_package(&WorkloadPackage::new(package.clone()))
        .await
        .unwrap();

    let result = hyper
        .load_workload_package(&WorkloadPackage::new(package))
        .await;
    assert!(
        matches!(result, Err(HyperError::Runtime(ref msg)) if msg.contains("already loaded a workload")),
        "second load should be rejected by Hyper one-shot workload contract"
    );
}

#[tokio::test]
async fn load_workload_package_rejects_invalid_target() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let manifest = actr_pack::PackageManifest {
        manufacturer: "test-mfr".to_string(),
        name: "BrokenActor".to_string(),
        version: "1.0.0".to_string(),
        binary: actr_pack::BinaryEntry {
            path: "bin/actor.wasm".to_string(),
            target: "invalid-target".to_string(),
            hash: String::new(),
            size: None,
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: None,
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        metadata: actr_pack::ManifestMetadata::default(),
    };
    let package = actr_pack::pack(&actr_pack::PackOptions {
        manifest,
        binary_bytes: minimal_wasm(),
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        signing_key: signing_key.clone(),
    })
    .unwrap();

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    let result = hyper
        .load_workload_package(&WorkloadPackage::new(package))
        .await;
    assert!(
        matches!(result, Err(HyperError::InvalidManifest(ref msg)) if msg.contains("unsupported binary target")),
        "invalid target should be rejected, got: {result:?}"
    );
}
