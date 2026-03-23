//! Integration tests: production mode MFR certificate cache + verify_package full flow
//!
//! Covered scenarios:
//! 1. Cache miss -> HTTP fetch MFR public key -> verification passes
//! 2. Cache hit -> no HTTP triggered -> verification passes
//! 3. MFR not registered -> AIS returns 404 -> UntrustedManufacturer
//! 4. Different MFRs -> independent caches
//! 5. HTTP request body and response format validation

use actr_hyper::{Hyper, HyperConfig, HyperError, MfrCertCache, TrustMode, WorkloadPackage};
use base64::Engine;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn minimal_wasm() -> Vec<u8> {
    b"\0asm\x01\x00\x00\x00".to_vec()
}

/// Production mode HyperConfig pointing to a mock AIS endpoint
fn prod_config(dir: &TempDir, ais_endpoint: &str) -> HyperConfig {
    HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Production {
        ais_endpoint: ais_endpoint.to_string(),
    })
}

/// Build a signed .actr package for the given manufacturer
fn make_signed_package(
    manufacturer: &str,
    actr_name: &str,
    version: &str,
    signing_key: &SigningKey,
) -> Vec<u8> {
    let wasm = minimal_wasm();
    let manifest = actr_pack::PackageManifest {
        manufacturer: manufacturer.to_string(),
        name: actr_name.to_string(),
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
        binary_bytes: wasm,
        resources: vec![],
        signing_key: signing_key.clone(),
    };
    actr_pack::pack(&opts).unwrap()
}

/// Build actrix MFR verifying_key response body
fn verifying_key_response(verifying_key: &ed25519_dalek::VerifyingKey) -> String {
    let key_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());
    format!(r#"{{"public_key":"{key_b64}"}}"#)
}

// ─── Test cases ─────────────────────────────────────────────────────────────────

/// Scenario 1: production mode, first verification -> fetch MFR public key from AIS -> passes
#[tokio::test]
async fn production_mode_fetches_mfr_key_and_verifies() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/mfr/acme/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(verifying_key_response(&verifying_key))
        .expect(1) // called exactly once
        .create_async()
        .await;

    let package = make_signed_package("acme", "Sensor", "1.0.0", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url())).await.unwrap();

    let manifest = hyper
        .verify_package(&WorkloadPackage::new(package))
        .await
        .unwrap();

    mock.assert_async().await;
    assert_eq!(manifest.manufacturer, "acme");
    assert_eq!(manifest.actr_name, "Sensor");
    assert_eq!(manifest.version, "1.0.0");
}

/// Scenario 2: two consecutive verifications for the same manufacturer -> second uses cache, no HTTP
#[tokio::test]
async fn production_mode_caches_mfr_key_on_second_verify() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/mfr/cached-mfr/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(verifying_key_response(&verifying_key))
        .expect(1) // called exactly once; second time uses cache
        .create_async()
        .await;

    let package = make_signed_package("cached-mfr", "App", "1.0.0", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url())).await.unwrap();

    // First: miss -> HTTP
    hyper
        .verify_package(&WorkloadPackage::new(package.clone()))
        .await
        .unwrap();
    // Second: hit -> no HTTP
    hyper
        .verify_package(&WorkloadPackage::new(package))
        .await
        .unwrap();

    mock.assert_async().await; // verify it was called only once
}

/// Scenario 3: MFR not registered (AIS returns 404) -> UntrustedManufacturer
#[tokio::test]
async fn production_mode_returns_untrusted_for_unknown_mfr() {
    let signing_key = SigningKey::generate(&mut OsRng);

    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/mfr/unknown-mfr/verifying_key")
        .with_status(404)
        .create_async()
        .await;

    let package = make_signed_package("unknown-mfr", "App", "1.0.0", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url())).await.unwrap();

    let result = hyper.verify_package(&WorkloadPackage::new(package)).await;

    assert!(
        matches!(result, Err(HyperError::UntrustedManufacturer(_))),
        "unknown MFR should return UntrustedManufacturer, got: {result:?}"
    );
}

/// Scenario 4: correct MFR public key -> passes; wrong key (cached) -> signature mismatch
#[tokio::test]
async fn production_mode_rejects_wrong_cached_key() {
    let real_signing_key = SigningKey::generate(&mut OsRng);
    let wrong_key = SigningKey::generate(&mut OsRng); // different key

    let mut server = mockito::Server::new_async().await;

    // AIS returns wrong_key's public key
    server
        .mock("GET", "/mfr/mfr-x/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(verifying_key_response(&wrong_key.verifying_key()))
        .create_async()
        .await;

    // Package signed with real_signing_key
    let package = make_signed_package("mfr-x", "X", "1.0.0", &real_signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url())).await.unwrap();

    let result = hyper.verify_package(&WorkloadPackage::new(package)).await;

    assert!(
        matches!(result, Err(HyperError::SignatureVerificationFailed(_))),
        "wrong public key should return SignatureVerificationFailed, got: {result:?}"
    );
}

/// Scenario 5: two different MFRs with independent keys -> each verifies independently
#[tokio::test]
async fn production_mode_independent_caches_per_manufacturer() {
    let key_a = SigningKey::generate(&mut OsRng);
    let key_b = SigningKey::generate(&mut OsRng);

    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/mfr/mfr-a/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(verifying_key_response(&key_a.verifying_key()))
        .create_async()
        .await;
    server
        .mock("GET", "/mfr/mfr-b/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(verifying_key_response(&key_b.verifying_key()))
        .create_async()
        .await;

    let pkg_a = make_signed_package("mfr-a", "ActorA", "1.0.0", &key_a);
    let pkg_b = make_signed_package("mfr-b", "ActorB", "1.0.0", &key_b);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url())).await.unwrap();

    let manifest_a = hyper
        .verify_package(&WorkloadPackage::new(pkg_a))
        .await
        .unwrap();
    let manifest_b = hyper
        .verify_package(&WorkloadPackage::new(pkg_b))
        .await
        .unwrap();

    assert_eq!(manifest_a.manufacturer, "mfr-a");
    assert_eq!(manifest_b.manufacturer, "mfr-b");
}

/// Standalone MfrCertCache test: get_from_cache is synchronously readable after prefetch
#[tokio::test]
async fn cert_cache_get_from_cache_after_prefetch() {
    use ed25519_dalek::VerifyingKey;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/mfr/sync-mfr/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(verifying_key_response(&verifying_key))
        .create_async()
        .await;

    let cache = MfrCertCache::new(server.url());

    // get_from_cache returns None before prefetch
    let before = cache.get_from_cache("sync-mfr");
    assert!(before.is_none(), "cache should be empty before prefetch");

    // prefetch
    cache.get_or_fetch("sync-mfr").await.unwrap();

    // get_from_cache now synchronously returns the public key
    let after: Option<VerifyingKey> = cache.get_from_cache("sync-mfr");
    assert!(after.is_some(), "cache should hit after prefetch");
    assert_eq!(
        after.unwrap().to_bytes(),
        verifying_key.to_bytes(),
        "cached public key should match the signing key"
    );
}

/// Non-.actr bytes -> InvalidManifest (no HTTP triggered)
#[tokio::test]
async fn production_mode_no_http_for_unknown_format() {
    let server = mockito::Server::new_async().await;
    // No mock endpoints set; if HTTP is triggered, the test fails

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url())).await.unwrap();

    let result = hyper
        .verify_package(&WorkloadPackage::new(b"this is not a package".to_vec()))
        .await;
    assert!(
        matches!(result, Err(HyperError::InvalidManifest(_))),
        "unknown format should return InvalidManifest, got: {result:?}"
    );
}
