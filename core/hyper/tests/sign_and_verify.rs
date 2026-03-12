//! Integration tests: .actr package sign → Hyper::verify_package full flow
//!
//! Scenarios:
//! 1. .actr package: pack → verify → manifest fields match
//! 2. .actr package: tampered binary → hash mismatch
//! 3. .actr package: wrong key → signature verification failed
//! 4. Unsigned bytes → InvalidManifest (unrecognized format)

use actr_hyper::{Hyper, HyperConfig, HyperError, TrustMode};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── Utility functions ───────────────────────────────────────────────────────

fn minimal_wasm() -> Vec<u8> {
    b"\0asm\x01\x00\x00\x00".to_vec()
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
        resources: vec![],
        metadata: actr_pack::ManifestMetadata::default(),
    };
    let opts = actr_pack::PackOptions {
        manifest,
        binary_bytes: binary.to_vec(),
        resources: vec![],
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

    let manifest = hyper.verify_package(&package).await.unwrap();
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
    let result = hyper.verify_package(&tampered).await;
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
    let result = hyper.verify_package(&package).await;
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

    let result = hyper.verify_package(b"this is not a binary").await;
    assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
}
