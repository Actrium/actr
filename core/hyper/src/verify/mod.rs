pub mod cert_cache;
pub mod embed;
pub mod manifest;

pub use cert_cache::MfrCertCache;
pub use embed::{embed_elf_manifest, embed_macho_manifest, embed_wasm_manifest};
pub use manifest::PackageManifest;

use std::sync::Arc;

use ed25519_dalek::{Signature, VerifyingKey};

use crate::config::TrustMode;
use crate::error::{HyperError, HyperResult};
use manifest::{
    extract_elf_manifest, extract_macho_manifest, extract_wasm_manifest,
    elf_binary_hash_excluding_manifest, macho_binary_hash_excluding_manifest,
    wasm_binary_hash_excluding_manifest, is_elf, is_macho, is_wasm,
};

/// 包验证器
///
/// 持有当前信任根（actrix 根 CA 或本地自签名公钥），
/// 对外提供统一的 `verify` 入口，内部按包格式分发。
pub struct PackageVerifier {
    trust_mode: TrustMode,
    /// 生产模式下的 MFR 公钥缓存（Development 模式为 None）
    cert_cache: Option<Arc<MfrCertCache>>,
}

impl PackageVerifier {
    pub fn new(trust_mode: TrustMode) -> Self {
        let cert_cache = match &trust_mode {
            TrustMode::Production { ais_endpoint } => {
                Some(MfrCertCache::new(ais_endpoint.clone()))
            }
            TrustMode::Development { .. } => None,
        };
        Self { trust_mode, cert_cache }
    }

    /// 验证包字节流，返回已验证的 manifest
    ///
    /// 流程：
    /// 1. 识别包格式（WASM / ELF / Mach-O）
    /// 2. 提取 manifest section
    /// 3. 重算 binary_hash（去掉 manifest section）
    /// 4. 验证 hash 一致性
    /// 5. 验证 MFR 签名
    pub fn verify(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        if is_wasm(bytes) {
            self.verify_wasm(bytes)
        } else if is_elf(bytes) {
            self.verify_elf(bytes)
        } else if is_macho(bytes) {
            self.verify_macho(bytes)
        } else {
            tracing::warn!("无法识别的包格式，不是 WASM/ELF/Mach-O");
            Err(HyperError::InvalidManifest(
                "不支持的包格式（仅支持 WASM、ELF64 LE、Mach-O 64-bit LE）".to_string(),
            ))
        }
    }

    fn verify_wasm(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // 1. 提取 manifest section
        let section_bytes = extract_wasm_manifest(bytes)
            .ok_or(HyperError::ManifestNotFound)?;

        // 2. 反序列化 manifest
        let manifest: PackageManifest = parse_manifest(section_bytes)?;

        // 3. 重算 binary_hash
        let computed_hash = wasm_binary_hash_excluding_manifest(bytes)?;

        // 4. 验证 hash 一致性
        if computed_hash != manifest.binary_hash {
            tracing::warn!(
                actr_type = manifest.actr_type_str(),
                "binary_hash 不匹配，包可能已被篡改"
            );
            return Err(HyperError::BinaryHashMismatch);
        }

        // 5. 验证 MFR 签名
        let pubkey = self.resolve_mfr_pubkey(&manifest.manufacturer)?;
        verify_manifest_signature(&manifest, &pubkey)?;

        tracing::info!(
            actr_type = manifest.actr_type_str(),
            "WASM 包签名验证通过"
        );
        Ok(manifest)
    }

    /// 验证 ELF 包（Native Mode 1 / Process Mode 2）
    ///
    /// 流程与 verify_wasm 一致，仅 section 提取和 hash 计算使用 ELF 实现。
    fn verify_elf(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // 1. 提取 manifest section
        let section_bytes = extract_elf_manifest(bytes)
            .ok_or(HyperError::ManifestNotFound)?;

        // 2. 反序列化 manifest
        let manifest: PackageManifest = parse_manifest(section_bytes)?;

        // 3. 重算 binary_hash（零填充 manifest section 后计算）
        let computed_hash = elf_binary_hash_excluding_manifest(bytes)?;

        // 4. 验证 hash 一致性
        if computed_hash != manifest.binary_hash {
            tracing::warn!(
                actr_type = manifest.actr_type_str(),
                "ELF binary_hash 不匹配，包可能已被篡改"
            );
            return Err(HyperError::BinaryHashMismatch);
        }

        // 5. 验证 MFR 签名
        let pubkey = self.resolve_mfr_pubkey(&manifest.manufacturer)?;
        verify_manifest_signature(&manifest, &pubkey)?;

        tracing::info!(
            actr_type = manifest.actr_type_str(),
            "ELF 包签名验证通过"
        );
        Ok(manifest)
    }

    /// 验证 Mach-O 包（Native Mode 1 / Process Mode 2）
    ///
    /// 流程与 verify_wasm 一致，仅 section 提取和 hash 计算使用 Mach-O 实现。
    /// fat binary 会在 extract_macho_manifest 阶段返回 ManifestNotFound。
    fn verify_macho(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // 1. 提取 manifest section（fat binary 会返回 None → ManifestNotFound）
        let section_bytes = extract_macho_manifest(bytes)
            .ok_or(HyperError::ManifestNotFound)?;

        // 2. 反序列化 manifest
        let manifest: PackageManifest = parse_manifest(section_bytes)?;

        // 3. 重算 binary_hash（零填充 manifest section 后计算）
        let computed_hash = macho_binary_hash_excluding_manifest(bytes)?;

        // 4. 验证 hash 一致性
        if computed_hash != manifest.binary_hash {
            tracing::warn!(
                actr_type = manifest.actr_type_str(),
                "Mach-O binary_hash 不匹配，包可能已被篡改"
            );
            return Err(HyperError::BinaryHashMismatch);
        }

        // 5. 验证 MFR 签名
        let pubkey = self.resolve_mfr_pubkey(&manifest.manufacturer)?;
        verify_manifest_signature(&manifest, &pubkey)?;

        tracing::info!(
            actr_type = manifest.actr_type_str(),
            "Mach-O 包签名验证通过"
        );
        Ok(manifest)
    }

    /// 获取 MFR 对应的 Ed25519 公钥（同步，仅用于缓存命中路径）
    ///
    /// - 开发模式：直接使用本地自签名公钥
    /// - 生产模式：从 cert_cache 读取（需先通过 `Hyper::prefetch_mfr_cert` 预热）
    fn resolve_mfr_pubkey(&self, manufacturer: &str) -> HyperResult<VerifyingKey> {
        match &self.trust_mode {
            TrustMode::Development { self_signed_pubkey } => {
                let bytes: [u8; 32] = self_signed_pubkey
                    .as_slice()
                    .try_into()
                    .map_err(|_| {
                        HyperError::Config(
                            "自签名公钥必须为 32 字节的 Ed25519 verifying key".to_string(),
                        )
                    })?;
                VerifyingKey::from_bytes(&bytes).map_err(|e| {
                    HyperError::Config(format!("自签名公钥无效: {e}"))
                })
            }
            TrustMode::Production { .. } => {
                // 生产模式：cert_cache 在此路径下保证已预热（由 Hyper::verify_package async 完成）
                // get_from_cache 是同步调用（std::sync::RwLock，无 HTTP）
                let cache = self
                    .cert_cache
                    .as_ref()
                    .expect("生产模式下 cert_cache 不应为 None");
                cache.get_from_cache(manufacturer).ok_or_else(|| {
                    HyperError::UntrustedManufacturer(format!(
                        "MFR 公钥未在缓存中，manufacturer={manufacturer}（需先调用 Hyper::verify_package）"
                    ))
                })
            }
        }
    }

    /// 生产模式：预取 MFR 公钥（异步 HTTP）并写入 cert_cache
    ///
    /// 由 `Hyper::verify_package_async` 在调用同步 verify 前调用。
    pub async fn prefetch_mfr_cert(&self, manufacturer: &str) -> HyperResult<()> {
        if let Some(cache) = &self.cert_cache {
            cache.get_or_fetch(manufacturer).await?;
        }
        Ok(())
    }
}

/// 验证 manifest 的 MFR 签名
///
/// 签名对象：manifest 中除 `signature` 字段以外的所有字段序列化后的字节。
fn verify_manifest_signature(
    manifest: &PackageManifest,
    pubkey: &VerifyingKey,
) -> HyperResult<()> {
    let signed_bytes = manifest_signed_bytes(manifest);

    let sig_bytes: [u8; 64] = manifest.signature.as_slice().try_into().map_err(|_| {
        HyperError::SignatureVerificationFailed(
            "签名长度不正确，Ed25519 签名必须为 64 字节".to_string(),
        )
    })?;
    let signature = Signature::from_bytes(&sig_bytes);

    pubkey
        .verify_strict(&signed_bytes, &signature)
        .map_err(|e| {
            HyperError::SignatureVerificationFailed(format!("Ed25519 签名验证失败: {e}"))
        })
}

/// 序列化 manifest 中需要被签名的字段
///
/// 不含 `signature` 字段本身，避免循环依赖。
/// CLI 签名工具需要保持与此函数完全一致的字节序列。
pub fn manifest_signed_bytes(manifest: &PackageManifest) -> Vec<u8> {
    // 简单拼接：各字段用 null byte 分隔，保持确定性
    let mut buf = Vec::new();
    buf.extend_from_slice(manifest.manufacturer.as_bytes());
    buf.push(0);
    buf.extend_from_slice(manifest.actr_name.as_bytes());
    buf.push(0);
    buf.extend_from_slice(manifest.version.as_bytes());
    buf.push(0);
    buf.extend_from_slice(&manifest.binary_hash);
    buf.push(0);
    for cap in &manifest.capabilities {
        buf.extend_from_slice(cap.as_bytes());
        buf.push(0);
    }
    buf
}

/// 解析 manifest section 字节为 `PackageManifest`
///
/// 当前使用 JSON 编码，后续可替换为更紧凑的格式。
fn parse_manifest(bytes: &[u8]) -> HyperResult<PackageManifest> {
    // manifest JSON 格式（供参考）：
    // {
    //   "manufacturer": "acme",
    //   "actr_name": "Sensor",
    //   "version": "1.0.0",
    //   "binary_hash": "<hex>",
    //   "capabilities": ["storage", "network"],
    //   "signature": "<base64>"
    // }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| HyperError::InvalidManifest(format!("JSON 解析失败: {e}")))?;

    let get_str = |key: &str| -> HyperResult<String> {
        value[key]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| HyperError::InvalidManifest(format!("字段 `{key}` 缺失或类型错误")))
    };

    let manufacturer = get_str("manufacturer")?;
    let actr_name = get_str("actr_name")?;
    let version = get_str("version")?;

    let hash_hex = get_str("binary_hash")?;
    let hash_bytes = hex_to_32_bytes(&hash_hex)?;

    let capabilities = value["capabilities"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let sig_b64 = get_str("signature")?;
    let signature = base64_decode(&sig_b64)?;

    Ok(PackageManifest {
        manufacturer,
        actr_name,
        version,
        binary_hash: hash_bytes,
        capabilities,
        signature,
    })
}

fn hex_to_32_bytes(hex: &str) -> HyperResult<[u8; 32]> {
    if hex.len() != 64 {
        return Err(HyperError::InvalidManifest(
            "binary_hash 必须为 64 位 hex 字符串（32 字节）".to_string(),
        ));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk)
            .map_err(|_| HyperError::InvalidManifest("binary_hash 包含非 UTF-8 字符".to_string()))?;
        out[i] = u8::from_str_radix(s, 16)
            .map_err(|_| HyperError::InvalidManifest("binary_hash 包含非法 hex 字符".to_string()))?;
    }
    Ok(out)
}

fn base64_decode(s: &str) -> HyperResult<Vec<u8>> {
    // 使用标准库以外的 base64 解码，暂用简单实现
    // TODO: 后续引入 base64 crate workspace dep
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    base64_simple_decode(&cleaned)
        .ok_or_else(|| HyperError::InvalidManifest("signature base64 解码失败".to_string()))
}

/// 极简 base64 解码（标准字母表，无 padding 容忍）
fn base64_simple_decode(s: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut decode_table = [0xFF_u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        decode_table[c as usize] = i as u8;
    }

    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        let [a, b, c, d] = [
            decode_table[bytes[i] as usize],
            decode_table[bytes[i + 1] as usize],
            decode_table[bytes[i + 2] as usize],
            decode_table[bytes[i + 3] as usize],
        ];
        if a == 0xFF || b == 0xFF || c == 0xFF || d == 0xFF {
            return None;
        }
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
        out.push((c << 6) | d);
        i += 4;
    }
    // 剩余 2 或 3 个字符
    let rem = bytes.len() - i;
    if rem == 2 {
        let [a, b] = [
            decode_table[bytes[i] as usize],
            decode_table[bytes[i + 1] as usize],
        ];
        if a == 0xFF || b == 0xFF {
            return None;
        }
        out.push((a << 2) | (b >> 4));
    } else if rem == 3 {
        let [a, b, c] = [
            decode_table[bytes[i] as usize],
            decode_table[bytes[i + 1] as usize],
            decode_table[bytes[i + 2] as usize],
        ];
        if a == 0xFF || b == 0xFF || c == 0xFF {
            return None;
        }
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
    }
    Some(out)
}

