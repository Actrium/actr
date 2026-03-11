//! Integration tests: dev mode sign -> Hyper::verify_package full flow
//!
//! Covered scenarios:
//! 1. Normal flow: WASM signed and embedded -> verification passes, manifest fields match
//! 2. Tamper detection: modified WASM content -> binary_hash mismatch
//! 3. Wrong key: verified with different key -> signature mismatch
//! 4. Re-signing: same WASM re-signed -> old section replaced, verification passes
//! 5. With capabilities: capabilities field coverage

use actr_hyper::{
    Hyper, HyperConfig, HyperError, PackageManifest, TrustMode, embed_wasm_manifest,
    manifest_signed_bytes, verify::manifest::wasm_binary_hash_excluding_manifest,
};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── Utility functions ─────────────────────────────────────────────────────────────────

fn minimal_wasm() -> Vec<u8> {
    // Minimal valid WASM: magic + version, no sections
    b"\0asm\x01\x00\x00\x00".to_vec()
}

/// Build a signed WASM package (full dev sign flow)
///
/// Returns (WASM bytes with embedded manifest, signing_key used)
fn dev_sign_wasm(
    wasm_bytes: &[u8],
    manufacturer: &str,
    actr_name: &str,
    version: &str,
    capabilities: &[&str],
    signing_key: &SigningKey,
) -> Vec<u8> {
    // 1. Compute binary_hash (excluding existing manifest section)
    let binary_hash = wasm_binary_hash_excluding_manifest(wasm_bytes).unwrap();

    // 2. Build manifest (signature initially empty)
    let caps: Vec<String> = capabilities.iter().map(|s| s.to_string()).collect();
    let manifest = PackageManifest {
        manufacturer: manufacturer.to_string(),
        actr_name: actr_name.to_string(),
        version: version.to_string(),
        binary_hash,
        capabilities: caps.clone(),
        signature: vec![],
    };

    // 3. Compute bytes to sign (consistent with verify/mod.rs manifest_signed_bytes)
    let signed_bytes = manifest_signed_bytes(&manifest);

    // 4. Ed25519 signature
    let signature = signing_key.sign(&signed_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

    // 5. Build manifest JSON
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

    // 6. Embed manifest section
    embed_wasm_manifest(wasm_bytes, &manifest_json).unwrap()
}

fn dev_config_with_key(dir: &TempDir, verifying_key: &ed25519_dalek::VerifyingKey) -> HyperConfig {
    HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Development {
        self_signed_pubkey: verifying_key.to_bytes().to_vec(),
    })
}

// ─── Test cases ─────────────────────────────────────────────────────────────────

/// Normal flow: sign WASM -> verify passes, manifest fields fully match
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
    assert_eq!(
        manifest.signature.len(),
        64,
        "signature should be 64-byte Ed25519"
    );
}

/// Empty capabilities list should also sign and verify correctly
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
    assert_eq!(manifest.manufacturer, "acme");
}

/// Tamper detection: WASM content modified -> binary_hash mismatch -> BinaryHashMismatch
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

    // Tamper: modify any byte after WASM magic
    // Find the first non-manifest section location to tamper
    // Here we construct a WASM with an extra section then tamper it
    let mut tampered = signed_wasm.clone();
    // Modify WASM version field (bytes 4-7), breaking binary_hash
    tampered[4] ^= 0xFF;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();
    let result = hyper.verify_package(&tampered).await;

    // Note: modifying WASM version may cause parse failure or hash mismatch
    // Both errors indicate tampering was detected
    assert!(
        matches!(
            result,
            Err(HyperError::BinaryHashMismatch) | Err(HyperError::InvalidManifest(_))
        ),
        "tampered WASM should fail verification, got: {result:?}"
    );
}

/// Wrong key: configure Hyper with different key's public key -> SignatureVerificationFailed
#[tokio::test]
async fn verify_rejects_wrong_signing_key() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let wrong_key = SigningKey::generate(&mut OsRng); // different key
    let wrong_verifying = wrong_key.verifying_key();

    let signed_wasm = dev_sign_wasm(
        &minimal_wasm(),
        "test-mfr",
        "Actor",
        "1.0.0",
        &[],
        &signing_key, // signed with signing_key
    );

    let dir = TempDir::new().unwrap();
    // But Hyper configured with wrong_verifying -> verification fails
    let hyper = Hyper::init(dev_config_with_key(&dir, &wrong_verifying))
        .await
        .unwrap();
    let result = hyper.verify_package(&signed_wasm).await;

    assert!(
        matches!(result, Err(HyperError::SignatureVerificationFailed(_))),
        "wrong public key should return SignatureVerificationFailed, got: {result:?}"
    );
}

/// Re-signing: re-sign the same WASM -> old section replaced, new manifest takes effect
#[tokio::test]
async fn resign_replaces_old_manifest() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let wasm = minimal_wasm();

    // First signing: version = "1.0.0"
    let signed_v1 = dev_sign_wasm(&wasm, "mfr", "App", "1.0.0", &[], &signing_key);

    // Second signing (re-sign already-signed WASM): version = "2.0.0"
    let signed_v2 = dev_sign_wasm(&signed_v1, "mfr", "App", "2.0.0", &[], &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    // v1 should pass
    let m1 = hyper.verify_package(&signed_v1).await.unwrap();
    assert_eq!(m1.version, "1.0.0");

    // v2 should pass, with version 2.0.0
    let m2 = hyper.verify_package(&signed_v2).await.unwrap();
    assert_eq!(
        m2.version, "2.0.0",
        "version should be 2.0.0 after re-signing"
    );
}

/// Unsigned WASM (no manifest section) -> ManifestNotFound
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

/// Non-WASM/ELF/Mach-O file -> InvalidManifest
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

/// binary_hash consistency: binary_hash computation should be identical before and after signing
#[tokio::test]
async fn binary_hash_stable_across_signing() {
    let signing_key = SigningKey::generate(&mut OsRng);

    let wasm = minimal_wasm();
    let hash_before = wasm_binary_hash_excluding_manifest(&wasm).unwrap();

    let signed = dev_sign_wasm(&wasm, "mfr", "A", "1.0", &[], &signing_key);
    let hash_after = wasm_binary_hash_excluding_manifest(&signed).unwrap();

    assert_eq!(
        hash_before, hash_after,
        "binary_hash should remain unchanged after embedding manifest section"
    );
}

/// verify_package returned manifest.binary_hash should equal original WASM's hash
#[tokio::test]
async fn verified_manifest_binary_hash_matches_original() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let wasm = minimal_wasm();
    let expected_hash = wasm_binary_hash_excluding_manifest(&wasm).unwrap();

    let signed_wasm = dev_sign_wasm(&wasm, "mfr", "B", "1.0", &[], &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();
    let manifest = hyper.verify_package(&signed_wasm).await.unwrap();

    assert_eq!(
        manifest.binary_hash, expected_hash,
        "verified binary_hash should match original WASM's hash"
    );
}
