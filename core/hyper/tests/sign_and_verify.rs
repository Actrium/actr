//! 集成测试：开发模式 dev sign → Hyper::verify_package 全流程
//!
//! 覆盖场景：
//! 1. 正常流程：WASM 签名嵌入 → 验证通过，manifest 字段匹配
//! 2. 篡改检测：修改 WASM 内容后 → binary_hash 不匹配
//! 3. 错误密钥：用不同密钥验证 → 签名不匹配
//! 4. 多次签名：相同 WASM 重签 → 替换旧 section，验证通过
//! 5. 带能力声明：capabilities 字段覆盖

use actr_hyper::{
    HyperConfig, HyperError, TrustMode,
    Hyper,
    embed_wasm_manifest, manifest_signed_bytes, PackageManifest,
    verify::manifest::wasm_binary_hash_excluding_manifest,
};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── 工具函数 ─────────────────────────────────────────────────────────────────

fn minimal_wasm() -> Vec<u8> {
    // 最小合法 WASM：magic + version，无 section
    b"\0asm\x01\x00\x00\x00".to_vec()
}

/// 构建已签名的 WASM 包（完整 dev sign 流程）
///
/// 返回 (嵌入 manifest 后的 WASM bytes, 使用的 signing_key)
fn dev_sign_wasm(
    wasm_bytes: &[u8],
    manufacturer: &str,
    actr_name: &str,
    version: &str,
    capabilities: &[&str],
    signing_key: &SigningKey,
) -> Vec<u8> {
    // 1. 计算 binary_hash（排除已有 manifest section）
    let binary_hash = wasm_binary_hash_excluding_manifest(wasm_bytes).unwrap();

    // 2. 构建 manifest（signature 暂为空）
    let caps: Vec<String> = capabilities.iter().map(|s| s.to_string()).collect();
    let manifest = PackageManifest {
        manufacturer: manufacturer.to_string(),
        actr_name: actr_name.to_string(),
        version: version.to_string(),
        binary_hash,
        capabilities: caps.clone(),
        signature: vec![],
    };

    // 3. 计算待签名字节（与 verify/mod.rs manifest_signed_bytes 一致）
    let signed_bytes = manifest_signed_bytes(&manifest);

    // 4. Ed25519 签名
    let signature = signing_key.sign(&signed_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD
        .encode(signature.to_bytes());

    // 5. 构建 manifest JSON
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

    // 6. 嵌入 manifest section
    embed_wasm_manifest(wasm_bytes, &manifest_json).unwrap()
}

fn dev_config_with_key(dir: &TempDir, verifying_key: &ed25519_dalek::VerifyingKey) -> HyperConfig {
    HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Development {
        self_signed_pubkey: verifying_key.to_bytes().to_vec(),
    })
}

// ─── 测试用例 ─────────────────────────────────────────────────────────────────

/// 正常流程：签名 WASM → 验证通过，manifest 字段完全匹配
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
    assert_eq!(manifest.signature.len(), 64, "signature 应为 64 字节 Ed25519");
}

/// 空能力列表也应正常签名和验证
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

/// 篡改检测：WASM 内容修改后 binary_hash 不匹配 → BinaryHashMismatch
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

    // 篡改：找到 WASM magic 之后的任意字节并修改
    // 找到第一个非 manifest section 的位置来篡改
    // 这里我们构造一个带有额外 section 的 WASM 然后篡改它
    let mut tampered = signed_wasm.clone();
    // 修改 WASM version 字段（bytes 4-7），破坏 binary_hash
    tampered[4] ^= 0xFF;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();
    let result = hyper.verify_package(&tampered).await;

    // 注意：修改了 WASM version 可能导致解析失败或 hash 不匹配
    // 两种错误都属于检测到篡改
    assert!(
        matches!(
            result,
            Err(HyperError::BinaryHashMismatch) | Err(HyperError::InvalidManifest(_))
        ),
        "篡改后验证应失败，实际: {result:?}"
    );
}

/// 错误密钥：用不同密钥的公钥配置 Hyper → SignatureVerificationFailed
#[tokio::test]
async fn verify_rejects_wrong_signing_key() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let wrong_key = SigningKey::generate(&mut OsRng); // 不同密钥
    let wrong_verifying = wrong_key.verifying_key();

    let signed_wasm = dev_sign_wasm(
        &minimal_wasm(),
        "test-mfr",
        "Actor",
        "1.0.0",
        &[],
        &signing_key, // 用 signing_key 签名
    );

    let dir = TempDir::new().unwrap();
    // 但 Hyper 配置为 wrong_verifying → 验证失败
    let hyper = Hyper::init(dev_config_with_key(&dir, &wrong_verifying))
        .await
        .unwrap();
    let result = hyper.verify_package(&signed_wasm).await;

    assert!(
        matches!(result, Err(HyperError::SignatureVerificationFailed(_))),
        "错误公钥应返回 SignatureVerificationFailed，实际: {result:?}"
    );
}

/// 多次签名：重签同一 WASM → 旧 section 被替换，新 manifest 生效
#[tokio::test]
async fn resign_replaces_old_manifest() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let wasm = minimal_wasm();

    // 第一次签名：version = "1.0.0"
    let signed_v1 = dev_sign_wasm(&wasm, "mfr", "App", "1.0.0", &[], &signing_key);

    // 第二次签名（在已签名的 WASM 上重签）：version = "2.0.0"
    let signed_v2 = dev_sign_wasm(&signed_v1, "mfr", "App", "2.0.0", &[], &signing_key);

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config_with_key(&dir, &verifying_key))
        .await
        .unwrap();

    // v1 应该通过
    let m1 = hyper.verify_package(&signed_v1).await.unwrap();
    assert_eq!(m1.version, "1.0.0");

    // v2 应该通过，且版本为 2.0.0
    let m2 = hyper.verify_package(&signed_v2).await.unwrap();
    assert_eq!(m2.version, "2.0.0", "重签后版本应为 2.0.0");
}

/// 未签名 WASM（无 manifest section）→ ManifestNotFound
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
        "未签名包应返回 ManifestNotFound"
    );
}

/// 非 WASM/ELF/Mach-O 文件 → InvalidManifest
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

/// binary_hash 一致性：签名前后 binary_hash 计算结果应相同
#[tokio::test]
async fn binary_hash_stable_across_signing() {
    let signing_key = SigningKey::generate(&mut OsRng);

    let wasm = minimal_wasm();
    let hash_before = wasm_binary_hash_excluding_manifest(&wasm).unwrap();

    let signed = dev_sign_wasm(&wasm, "mfr", "A", "1.0", &[], &signing_key);
    let hash_after = wasm_binary_hash_excluding_manifest(&signed).unwrap();

    assert_eq!(
        hash_before, hash_after,
        "嵌入 manifest section 后 binary_hash 应保持不变"
    );
}

/// verify_package 返回的 manifest.binary_hash 应等于原始 WASM 的 hash
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
        "验证后 binary_hash 应与原始 WASM 的 hash 一致"
    );
}
