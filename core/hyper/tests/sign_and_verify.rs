//! Integration tests: package sign → Hyper::verify_package full flow
//!
//! Covers both the new `.actr` ZIP package format and legacy embedded-manifest WASM.
//!
//! Scenarios:
//! 1. .actr package: pack → verify → manifest fields match
//! 2. .actr package: tampered binary → hash mismatch
//! 3. .actr package: wrong key → signature verification failed
//! 4. Legacy WASM: sign and embed → verification passes
//! 5. Legacy WASM: tamper detection
//! 6. Legacy WASM: wrong key rejected
//! 7. Re-signing legacy WASM replaces old manifest
//! 8. Unsigned WASM → ManifestNotFound
//! 9. Unknown format → InvalidManifest

use actr_hyper::{
    Hyper, HyperConfig, HyperError, PackageManifest, TrustMode, embed_wasm_manifest,
    manifest_signed_bytes, verify::manifest::wasm_binary_hash_excluding_manifest,
};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── Utility functions ───────────────────────────────────────────────────────

fn minimal_wasm() -> Vec<u8> {
    b"\0asm\x01\x00\x00\x00".to_vec()
}

/// Build a signed WASM package with embedded manifest (legacy format)
fn dev_sign_wasm(
    wasm_bytes: &[u8],
    manufacturer: &str,
    actr_name: &str,
    version: &str,
    capabilities: &[&str],
    signing_key: &SigningKey,
) -> Vec<u8> {
    let binary_hash = wasm_binary_hash_excluding_manifest(wasm_bytes).unwrap();
    let caps: Vec<String> = capabilities.iter().map(|s| s.to_string()).collect();
    let manifest = PackageManifest {
        manufacturer: manufacturer.to_string(),
        actr_name: actr_name.to_string(),
        version: version.to_string(),
        binary_hash,
        capabilities: caps.clone(),
        signature: vec![],
    };
    let signed_bytes = manifest_signed_bytes(&manifest);
    let signature = signing_key.sign(&signed_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
    let hash_hex: String = binary_hash.iter().map(|b| format!("{b:02x}")).collect();
    let manifest_json = serde_json::to_vec(&serde_json::json!({
        "manufacturer": manufacturer,
        "actr_name": actr_name,
        "version": version,
        "binary_hash": hash_hex,
        "capabilities": caps,
        "signature": sig_b64,
    }))
    .unwrap();
    embed_wasm_manifest(wasm_bytes, &manifest_json).unwrap()
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
    if let Some(pos) = tampered
        .windows(13)
        .position(|w| w == b"original wasm")
    {
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

// ─── Legacy WASM tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn wasm_sign_then_verify_succeeds() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let wasm = minimal_wasm();
    let signed_wasm = dev_sign_wasm(
        &wasm,
        "test-mfr",
        "MyActor",
        "1.2.3",
        &["storage", "network"],
        &signing_key,
    );

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    let manifest = hyper.verify_package(&signed_wasm).await.unwrap();
    assert_eq!(manifest.manufacturer, "test-mfr");
    assert_eq!(manifest.actr_name, "MyActor");
    assert_eq!(manifest.version, "1.2.3");
    assert_eq!(manifest.capabilities, vec!["storage", "network"]);
}

#[tokio::test]
async fn wasm_sign_with_no_capabilities() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let signed_wasm = dev_sign_wasm(
        &minimal_wasm(),
        "acme",
        "Sensor",
        "0.1.0",
        &[],
        &signing_key,
    );

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();
    let manifest = hyper.verify_package(&signed_wasm).await.unwrap();
    assert_eq!(manifest.capabilities, Vec::<String>::new());
}

#[tokio::test]
async fn verify_detects_tampered_wasm_content() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let original_wasm = minimal_wasm();
    let signed_wasm = dev_sign_wasm(
        &original_wasm,
        "test-mfr",
        "SecureActor",
        "1.0.0",
        &[],
        &signing_key,
    );

    let mut tampered = signed_wasm.clone();
    tampered[4] ^= 0xFF;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();
    let result = hyper.verify_package(&tampered).await;
    assert!(
        matches!(
            result,
            Err(HyperError::BinaryHashMismatch) | Err(HyperError::InvalidManifest(_))
        ),
        "tampered WASM should fail verification, got: {result:?}"
    );
}

#[tokio::test]
async fn verify_rejects_wrong_signing_key() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let wrong_key = SigningKey::generate(&mut OsRng);
    let wrong_verifying = wrong_key.verifying_key();

    let signed_wasm = dev_sign_wasm(
        &minimal_wasm(),
        "test-mfr",
        "Actor",
        "1.0.0",
        &[],
        &signing_key,
    );

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &wrong_verifying))
        .await
        .unwrap();
    let result = hyper.verify_package(&signed_wasm).await;
    assert!(
        matches!(result, Err(HyperError::SignatureVerificationFailed(_))),
        "wrong public key should return SignatureVerificationFailed, got: {result:?}"
    );
}

#[tokio::test]
async fn resign_replaces_old_manifest() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let wasm = minimal_wasm();
    let signed_v1 = dev_sign_wasm(&wasm, "mfr", "App", "1.0.0", &[], &signing_key);
    let signed_v2 = dev_sign_wasm(&signed_v1, "mfr", "App", "2.0.0", &[], &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    let m1 = hyper.verify_package(&signed_v1).await.unwrap();
    assert_eq!(m1.version, "1.0.0");

    let m2 = hyper.verify_package(&signed_v2).await.unwrap();
    assert_eq!(m2.version, "2.0.0");
}

#[tokio::test]
async fn verify_rejects_wasm_without_manifest() {
    let dir = TempDir::new().unwrap();
    let signing_key = SigningKey::generate(&mut OsRng);
    let hyper = Hyper::init(dev_config_with_key(&dir, &signing_key.verifying_key()))
        .await
        .unwrap();

    let result = hyper.verify_package(&minimal_wasm()).await;
    assert!(
        matches!(result, Err(HyperError::ManifestNotFound)),
        "unsigned package should return ManifestNotFound"
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

#[tokio::test]
async fn binary_hash_stable_across_signing() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let wasm = minimal_wasm();
    let hash_before = wasm_binary_hash_excluding_manifest(&wasm).unwrap();
    let signed = dev_sign_wasm(&wasm, "mfr", "A", "1.0", &[], &signing_key);
    let hash_after = wasm_binary_hash_excluding_manifest(&signed).unwrap();
    assert_eq!(hash_before, hash_after);
}
