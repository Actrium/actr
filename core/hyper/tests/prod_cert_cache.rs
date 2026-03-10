//! 集成测试：生产模式 MFR 证书缓存 + verify_package 全流程
//!
//! 覆盖场景：
//! 1. 缓存 miss → HTTP 拉取 MFR 公钥 → 验证通过
//! 2. 缓存 hit → 不触发 HTTP → 验证通过
//! 3. MFR 未注册 → AIS 返回 404 → UntrustedManufacturer
//! 4. 不同 MFR → 各自独立缓存
//! 5. HTTP 请求体与响应格式验证

use actr_hyper::{
    HyperConfig, HyperError, TrustMode, Hyper, MfrCertCache,
    embed_wasm_manifest, manifest_signed_bytes, PackageManifest,
    verify::manifest::wasm_binary_hash_excluding_manifest,
};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── 辅助函数 ─────────────────────────────────────────────────────────────────

fn minimal_wasm() -> Vec<u8> {
    b"\0asm\x01\x00\x00\x00".to_vec()
}

/// 生产模式 HyperConfig，指向 mock AIS 端点
fn prod_config(dir: &TempDir, ais_endpoint: &str) -> HyperConfig {
    HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Production {
        ais_endpoint: ais_endpoint.to_string(),
    })
}

/// 为指定 manufacturer 构建已签名的 WASM，返回 (embedded_wasm, signing_key)
fn make_signed_wasm(
    manufacturer: &str,
    actr_name: &str,
    version: &str,
    signing_key: &SigningKey,
) -> Vec<u8> {
    let wasm = minimal_wasm();
    let binary_hash = wasm_binary_hash_excluding_manifest(&wasm).unwrap();

    let manifest = PackageManifest {
        manufacturer: manufacturer.to_string(),
        actr_name: actr_name.to_string(),
        version: version.to_string(),
        binary_hash,
        capabilities: vec![],
        signature: vec![],
    };

    let signed_bytes = manifest_signed_bytes(&manifest);
    let sig = signing_key.sign(&signed_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    let hash_hex: String = binary_hash.iter().map(|b| format!("{b:02x}")).collect();

    let json = serde_json::to_vec(&serde_json::json!({
        "manufacturer": manufacturer,
        "actr_name": actr_name,
        "version": version,
        "binary_hash": hash_hex,
        "capabilities": [],
        "signature": sig_b64,
    }))
    .unwrap();

    embed_wasm_manifest(&wasm, &json).unwrap()
}

/// 构建 actrix MFR verifying_key 响应 body
fn verifying_key_response(verifying_key: &ed25519_dalek::VerifyingKey) -> String {
    let key_b64 = base64::engine::general_purpose::STANDARD
        .encode(verifying_key.to_bytes());
    format!(r#"{{"public_key":"{key_b64}"}}"#)
}

// ─── 测试用例 ─────────────────────────────────────────────────────────────────

/// 场景 1：生产模式，首次验证 → 从 AIS 拉取 MFR 公钥 → 验证通过
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
        .expect(1) // 只调用一次
        .create_async()
        .await;

    let signed_wasm = make_signed_wasm("acme", "Sensor", "1.0.0", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url()))
        .await
        .unwrap();

    let manifest = hyper.verify_package(&signed_wasm).await.unwrap();

    mock.assert_async().await;
    assert_eq!(manifest.manufacturer, "acme");
    assert_eq!(manifest.actr_name, "Sensor");
    assert_eq!(manifest.version, "1.0.0");
}

/// 场景 2：连续两次验证同一 manufacturer → 第二次使用缓存，不触发 HTTP
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
        .expect(1) // 只调用一次，第二次走缓存
        .create_async()
        .await;

    let signed_wasm = make_signed_wasm("cached-mfr", "App", "1.0.0", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url()))
        .await
        .unwrap();

    // 第一次：miss → HTTP
    hyper.verify_package(&signed_wasm).await.unwrap();
    // 第二次：hit → no HTTP
    hyper.verify_package(&signed_wasm).await.unwrap();

    mock.assert_async().await; // 验证只调用了一次
}

/// 场景 3：MFR 未注册（AIS 返回 404）→ UntrustedManufacturer
#[tokio::test]
async fn production_mode_returns_untrusted_for_unknown_mfr() {
    let signing_key = SigningKey::generate(&mut OsRng);

    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/mfr/unknown-mfr/verifying_key")
        .with_status(404)
        .create_async()
        .await;

    let signed_wasm = make_signed_wasm("unknown-mfr", "App", "1.0.0", &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url()))
        .await
        .unwrap();

    let result = hyper.verify_package(&signed_wasm).await;

    assert!(
        matches!(result, Err(HyperError::UntrustedManufacturer(_))),
        "未知 MFR 应返回 UntrustedManufacturer，实际: {result:?}"
    );
}

/// 场景 4：正确 MFR 公钥 → 验证通过；错误公钥（已缓存）→ 签名不匹配
#[tokio::test]
async fn production_mode_rejects_wrong_cached_key() {
    let real_signing_key = SigningKey::generate(&mut OsRng);
    let wrong_key = SigningKey::generate(&mut OsRng); // 不同密钥

    let mut server = mockito::Server::new_async().await;

    // AIS 返回 wrong_key 的公钥
    server
        .mock("GET", "/mfr/mfr-x/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(verifying_key_response(&wrong_key.verifying_key()))
        .create_async()
        .await;

    // WASM 用 real_signing_key 签名
    let signed_wasm = make_signed_wasm("mfr-x", "X", "1.0.0", &real_signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url()))
        .await
        .unwrap();

    let result = hyper.verify_package(&signed_wasm).await;

    assert!(
        matches!(result, Err(HyperError::SignatureVerificationFailed(_))),
        "错误公钥应返回 SignatureVerificationFailed，实际: {result:?}"
    );
}

/// 场景 5：两个不同 MFR，各自有独立密钥 → 各自验证通过
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

    let wasm_a = make_signed_wasm("mfr-a", "ActorA", "1.0.0", &key_a);
    let wasm_b = make_signed_wasm("mfr-b", "ActorB", "1.0.0", &key_b);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url()))
        .await
        .unwrap();

    let manifest_a = hyper.verify_package(&wasm_a).await.unwrap();
    let manifest_b = hyper.verify_package(&wasm_b).await.unwrap();

    assert_eq!(manifest_a.manufacturer, "mfr-a");
    assert_eq!(manifest_b.manufacturer, "mfr-b");
}

/// 独立测试 MfrCertCache：get_from_cache 在预取后同步可读
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

    // get_from_cache 在预取前返回 None
    let before = cache.get_from_cache("sync-mfr");
    assert!(before.is_none(), "预取前缓存应为空");

    // 预取
    cache.get_or_fetch("sync-mfr").await.unwrap();

    // get_from_cache 现在同步返回公钥
    let after: Option<VerifyingKey> = cache.get_from_cache("sync-mfr");
    assert!(after.is_some(), "预取后缓存应命中");
    assert_eq!(
        after.unwrap().to_bytes(),
        verifying_key.to_bytes(),
        "缓存公钥应与签名密钥对应"
    );
}

/// 无 manifest section 的 WASM 在生产模式下不触发 HTTP（quick_extract 返回 None）
#[tokio::test]
async fn production_mode_no_http_for_unsigned_wasm() {
    let mut server = mockito::Server::new_async().await;
    // 不设置任何 mock endpoint，若触发 HTTP 则测试失败

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(prod_config(&dir, &server.url()))
        .await
        .unwrap();

    // 无 manifest section 的 WASM → 不会触发 HTTP（quick_extract 返回 None）
    // 直接进入 verify，返回 ManifestNotFound
    let result = hyper.verify_package(b"\0asm\x01\x00\x00\x00").await;
    assert!(
        matches!(result, Err(HyperError::ManifestNotFound)),
        "无 manifest 应返回 ManifestNotFound，实际: {result:?}"
    );
}
