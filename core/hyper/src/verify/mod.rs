pub mod manifest;

#[cfg(not(target_arch = "wasm32"))]
pub mod cert_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod embed;

#[cfg(not(target_arch = "wasm32"))]
pub use cert_cache::MfrCertCache;
#[cfg(not(target_arch = "wasm32"))]
pub use embed::{embed_elf_manifest, embed_macho_manifest, embed_wasm_manifest};
pub use manifest::PackageManifest;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use actr_platform_traits::CryptoProvider;
#[cfg(not(target_arch = "wasm32"))]
use ed25519_dalek::{Signature, VerifyingKey};

#[cfg(not(target_arch = "wasm32"))]
use crate::config::TrustMode;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::{HyperError, HyperResult};
#[cfg(not(target_arch = "wasm32"))]
use manifest::{
    elf_binary_hash_excluding_manifest, extract_elf_manifest, extract_macho_manifest,
    extract_wasm_manifest, is_elf, is_macho, is_wasm, macho_binary_hash_excluding_manifest,
    wasm_binary_hash_excluding_manifest,
};

#[cfg(not(target_arch = "wasm32"))]
/// Package verifier.
///
/// Holds the current trust root, either the Actrix root CA or a local self-signed public key,
/// and exposes a unified `verify` entry point that dispatches by package format.
///
/// When a `CryptoProvider` is injected, signature verification delegates to it
/// (enabling Web Crypto on browser). Otherwise falls back to direct ed25519-dalek.
pub struct PackageVerifier {
    trust_mode: TrustMode,
    /// Cache of MFR public keys in production mode. `None` in development mode.
    cert_cache: Option<Arc<MfrCertCache>>,
    /// Optional platform crypto provider for cross-platform signature verification
    crypto: Option<Arc<dyn CryptoProvider>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl PackageVerifier {
    pub fn new(trust_mode: TrustMode) -> Self {
        let cert_cache = match &trust_mode {
            TrustMode::Production { ais_endpoint } => Some(MfrCertCache::new(ais_endpoint.clone())),
            TrustMode::Development { .. } => None,
        };
        Self {
            trust_mode,
            cert_cache,
            crypto: None,
        }
    }

    /// Set a platform crypto provider for cross-platform signature verification
    pub fn with_crypto(mut self, crypto: Arc<dyn CryptoProvider>) -> Self {
        self.crypto = Some(crypto);
        self
    }

    /// Verify package bytes and return the validated manifest.
    ///
    /// Flow:
    /// 1. Detect the package format: WASM, ELF, or Mach-O
    /// 2. Extract the manifest section
    /// 3. Recompute `binary_hash` with the manifest section excluded
    /// 4. Verify hash consistency
    /// 5. Verify the MFR signature
    pub fn verify(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        if is_wasm(bytes) {
            self.verify_wasm(bytes)
        } else if is_elf(bytes) {
            self.verify_elf(bytes)
        } else if is_macho(bytes) {
            self.verify_macho(bytes)
        } else {
            tracing::warn!("Unrecognized package format, not WASM/ELF/Mach-O");
            Err(HyperError::InvalidManifest(
                "Unsupported package format; only WASM, ELF64 little-endian, and Mach-O 64-bit little-endian are supported".to_string(),
            ))
        }
    }

    fn verify_wasm(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // 1. Extract the manifest section.
        let section_bytes = extract_wasm_manifest(bytes).ok_or(HyperError::ManifestNotFound)?;

        // 2. Deserialize the manifest.
        let manifest: PackageManifest = parse_manifest(section_bytes)?;

        // 3. Recompute `binary_hash`.
        let computed_hash = wasm_binary_hash_excluding_manifest(bytes)?;

        // 4. Verify hash consistency.
        if computed_hash != manifest.binary_hash {
            tracing::warn!(
                actr_type = manifest.actr_type_str(),
                "binary_hash mismatch; the package may have been tampered with"
            );
            return Err(HyperError::BinaryHashMismatch);
        }

        // 5. Verify the MFR signature.
        let pubkey = self.resolve_mfr_pubkey(&manifest.manufacturer)?;
        self.verify_signature(&manifest, &pubkey)?;

        tracing::info!(
            actr_type = manifest.actr_type_str(),
            "WASM package signature verified"
        );
        Ok(manifest)
    }

    /// Verify an ELF package (Native / DynCLib).
    ///
    /// The flow matches `verify_wasm`, but section extraction and hashing use ELF-specific logic.
    fn verify_elf(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // 1. Extract the manifest section.
        let section_bytes = extract_elf_manifest(bytes).ok_or(HyperError::ManifestNotFound)?;

        // 2. Deserialize the manifest.
        let manifest: PackageManifest = parse_manifest(section_bytes)?;

        // 3. Recompute `binary_hash` after zero-filling the manifest section.
        let computed_hash = elf_binary_hash_excluding_manifest(bytes)?;

        // 4. Verify hash consistency.
        if computed_hash != manifest.binary_hash {
            tracing::warn!(
                actr_type = manifest.actr_type_str(),
                "ELF binary_hash mismatch; the package may have been tampered with"
            );
            return Err(HyperError::BinaryHashMismatch);
        }

        // 5. Verify the MFR signature.
        let pubkey = self.resolve_mfr_pubkey(&manifest.manufacturer)?;
        self.verify_signature(&manifest, &pubkey)?;

        tracing::info!(
            actr_type = manifest.actr_type_str(),
            "ELF package signature verified"
        );
        Ok(manifest)
    }

    /// Verify a Mach-O package (Native / DynCLib).
    ///
    /// The flow matches `verify_wasm`, but section extraction and hashing use Mach-O-specific logic.
    /// Fat binaries return `ManifestNotFound` from `extract_macho_manifest`.
    fn verify_macho(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // 1. Extract the manifest section. Fat binaries return `None` -> `ManifestNotFound`.
        let section_bytes = extract_macho_manifest(bytes).ok_or(HyperError::ManifestNotFound)?;

        // 2. Deserialize the manifest.
        let manifest: PackageManifest = parse_manifest(section_bytes)?;

        // 3. Recompute `binary_hash` after zero-filling the manifest section.
        let computed_hash = macho_binary_hash_excluding_manifest(bytes)?;

        // 4. Verify hash consistency.
        if computed_hash != manifest.binary_hash {
            tracing::warn!(
                actr_type = manifest.actr_type_str(),
                "Mach-O binary_hash mismatch; the package may have been tampered with"
            );
            return Err(HyperError::BinaryHashMismatch);
        }

        // 5. Verify the MFR signature.
        let pubkey = self.resolve_mfr_pubkey(&manifest.manufacturer)?;
        self.verify_signature(&manifest, &pubkey)?;

        tracing::info!(
            actr_type = manifest.actr_type_str(),
            "Mach-O package signature verified"
        );
        Ok(manifest)
    }

    /// Verify the MFR signature.
    ///
    /// Currently uses ed25519-dalek directly. The stored `CryptoProvider` will be
    /// used in Phase 4 when verification moves to a fully async path.
    fn verify_signature(
        &self,
        manifest: &PackageManifest,
        pubkey: &VerifyingKey,
    ) -> HyperResult<()> {
        verify_manifest_signature(manifest, pubkey)
    }

    /// Resolve the Ed25519 public key for the MFR synchronously, used only on cache-hit paths.
    ///
    /// - Development mode: use the local self-signed public key directly
    /// - Production mode: read from `cert_cache`, which must be warmed by `Hyper::prefetch_mfr_cert`
    fn resolve_mfr_pubkey(&self, manufacturer: &str) -> HyperResult<VerifyingKey> {
        match &self.trust_mode {
            TrustMode::Development { self_signed_pubkey } => {
                let bytes: [u8; 32] = self_signed_pubkey.as_slice().try_into().map_err(|_| {
                    HyperError::Config(
                        "The self-signed public key must be a 32-byte Ed25519 verifying key"
                            .to_string(),
                    )
                })?;
                VerifyingKey::from_bytes(&bytes)
                    .map_err(|e| HyperError::Config(format!("Invalid self-signed public key: {e}")))
            }
            TrustMode::Production { .. } => {
                // In production the cert cache is expected to be prewarmed by async verification.
                // `get_from_cache` is synchronous and performs no HTTP.
                let cache = self
                    .cert_cache
                    .as_ref()
                    .expect("cert_cache must not be None in production mode");
                cache.get_from_cache(manufacturer).ok_or_else(|| {
                    HyperError::UntrustedManufacturer(format!(
                        "MFR public key missing from cache for manufacturer={manufacturer}; call Hyper::verify_package first"
                    ))
                })
            }
        }
    }

    /// In production, prefetch the MFR public key over async HTTP and store it in `cert_cache`.
    ///
    /// Called by `Hyper::verify_package_async` before invoking synchronous verification.
    pub async fn prefetch_mfr_cert(&self, manufacturer: &str) -> HyperResult<()> {
        if let Some(cache) = &self.cert_cache {
            cache.get_or_fetch(manufacturer).await?;
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Verify the MFR signature embedded in the manifest.
///
/// The signed payload is the serialized bytes of all manifest fields except `signature`.
fn verify_manifest_signature(manifest: &PackageManifest, pubkey: &VerifyingKey) -> HyperResult<()> {
    let signed_bytes = manifest_signed_bytes(manifest);

    let sig_bytes: [u8; 64] = manifest.signature.as_slice().try_into().map_err(|_| {
        HyperError::SignatureVerificationFailed(
            "Invalid signature length; Ed25519 signatures must be 64 bytes".to_string(),
        )
    })?;
    let signature = Signature::from_bytes(&sig_bytes);

    pubkey
        .verify_strict(&signed_bytes, &signature)
        .map_err(|e| {
            HyperError::SignatureVerificationFailed(format!(
                "Ed25519 signature verification failed: {e}"
            ))
        })
}

#[cfg(not(target_arch = "wasm32"))]
/// Serialize the manifest fields that participate in signing.
///
/// This excludes the `signature` field itself to avoid circular dependence.
/// The CLI signing tool must produce exactly the same byte sequence.
pub fn manifest_signed_bytes(manifest: &PackageManifest) -> Vec<u8> {
    // Simple concatenation with null-byte separators keeps the layout deterministic.
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

#[cfg(not(target_arch = "wasm32"))]
/// Parse manifest section bytes into a `PackageManifest`.
///
/// JSON is used for now and can be replaced later by a more compact format.
fn parse_manifest(bytes: &[u8]) -> HyperResult<PackageManifest> {
    // Example manifest JSON format:
    // {
    //   "manufacturer": "acme",
    //   "actr_name": "Sensor",
    //   "version": "1.0.0",
    //   "binary_hash": "<hex>",
    //   "capabilities": ["storage", "network"],
    //   "signature": "<base64>"
    // }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| HyperError::InvalidManifest(format!("JSON parsing failed: {e}")))?;

    let get_str = |key: &str| -> HyperResult<String> {
        value[key].as_str().map(|s| s.to_string()).ok_or_else(|| {
            HyperError::InvalidManifest(format!("Field `{key}` is missing or has the wrong type"))
        })
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

#[cfg(not(target_arch = "wasm32"))]
fn hex_to_32_bytes(hex: &str) -> HyperResult<[u8; 32]> {
    if hex.len() != 64 {
        return Err(HyperError::InvalidManifest(
            "binary_hash must be a 64-character hex string (32 bytes)".to_string(),
        ));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|_| {
            HyperError::InvalidManifest("binary_hash contains non-UTF-8 characters".to_string())
        })?;
        out[i] = u8::from_str_radix(s, 16).map_err(|_| {
            HyperError::InvalidManifest("binary_hash contains invalid hex characters".to_string())
        })?;
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn base64_decode(s: &str) -> HyperResult<Vec<u8>> {
    // Use a small local base64 decoder for now instead of adding a dependency.
    // TODO: Replace this with the workspace base64 crate later.
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    base64_simple_decode(&cleaned)
        .ok_or_else(|| HyperError::InvalidManifest("Failed to decode signature base64".to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
/// Minimal base64 decoder using the standard alphabet without padding tolerance.
fn base64_simple_decode(s: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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
    // Handle the remaining 2 or 3 characters.
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
