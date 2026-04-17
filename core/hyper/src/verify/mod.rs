//! Package verification module.
//!
//! Verifies `.actr` ZIP STORE packages using `actr_pack`.

pub mod manifest;

#[cfg(not(target_arch = "wasm32"))]
pub mod cert_cache;

#[cfg(not(target_arch = "wasm32"))]
pub use cert_cache::MfrCertCache;
pub use manifest::PackageManifest;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use actr_platform_traits::CryptoProvider;
#[cfg(not(target_arch = "wasm32"))]
use ed25519_dalek::VerifyingKey;

#[cfg(not(target_arch = "wasm32"))]
use crate::config::TrustMode;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::{HyperError, HyperResult};

#[cfg(not(target_arch = "wasm32"))]
/// Package verifier.
///
/// Holds the current trust root, either the Actrix root CA or a local self-signed public key,
/// and exposes a unified `verify` entry point that delegates to `actr_pack`.
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
    /// Only `.actr` ZIP STORE packages are supported.
    pub fn verify(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        if !is_actr_package(bytes) {
            tracing::warn!("Unrecognized package format");
            return Err(HyperError::InvalidManifest(
                "Unsupported package format; expected .actr ZIP package".to_string(),
            ));
        }

        self.verify_actr_package(bytes)
    }

    /// Verify an `.actr` ZIP STORE package using `actr_pack`.
    fn verify_actr_package(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // Read manifest first to extract manufacturer for pubkey resolution
        let pack_manifest = actr_pack::read_manifest(bytes).map_err(|e| match &e {
            actr_pack::PackError::ManifestNotFound => HyperError::ManifestNotFound,
            actr_pack::PackError::ManifestParseError(msg) => {
                HyperError::InvalidManifest(msg.clone())
            }
            _ => HyperError::InvalidManifest(e.to_string()),
        })?;

        // Production mode: signing_key_id is mandatory (no degradation)
        if matches!(&self.trust_mode, TrustMode::Production { .. }) {
            if pack_manifest.signing_key_id.is_none() {
                return Err(HyperError::InvalidManifest(
                    "Package missing 'signing_key_id' in manifest. \
                     Rebuild with the latest 'actr build' command."
                        .to_string(),
                ));
            }
        }

        let pubkey = self.resolve_mfr_pubkey(
            &pack_manifest.manufacturer,
            pack_manifest.signing_key_id.as_deref(),
        )?;

        let verified = actr_pack::verify(bytes, &pubkey).map_err(|e| match &e {
            actr_pack::PackError::SignatureVerificationFailed(msg) => {
                HyperError::SignatureVerificationFailed(msg.clone())
            }
            actr_pack::PackError::BinaryHashMismatch { .. } => HyperError::BinaryHashMismatch,
            actr_pack::PackError::SignatureNotFound => HyperError::SignatureVerificationFailed(
                "signature not found in package".to_string(),
            ),
            actr_pack::PackError::BinaryNotFound(path) => {
                HyperError::InvalidManifest(format!("binary not found: {path}"))
            }
            actr_pack::PackError::ManifestNotFound => HyperError::ManifestNotFound,
            _ => HyperError::InvalidManifest(e.to_string()),
        })?;

        tracing::info!(
            actr_type = %verified.manifest.actr_type_str(),
            ".actr package verified"
        );

        // Convert actr_pack::VerifiedPackage to hyper's PackageManifest
        Ok(PackageManifest {
            manufacturer: verified.manifest.manufacturer,
            actr_name: verified.manifest.name,
            version: verified.manifest.version,
            binary_path: verified.manifest.binary.path,
            binary_target: verified.manifest.binary.target.clone(),
            binary_hash: hex_to_32_bytes(&verified.manifest.binary.hash).unwrap_or_default(),
            capabilities: vec![],
            signature: verified.sig_raw,
            manifest_raw: verified.manifest_raw,
            target: verified.manifest.binary.target,
        })
    }

    /// Resolve the Ed25519 public key for the MFR synchronously, used only on cache-hit paths.
    fn resolve_mfr_pubkey(
        &self,
        manufacturer: &str,
        key_id: Option<&str>,
    ) -> HyperResult<VerifyingKey> {
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
                let cache = self
                    .cert_cache
                    .as_ref()
                    .expect("cert_cache must not be None in production mode");
                cache.get_from_cache(manufacturer, key_id).ok_or_else(|| {
                    HyperError::UntrustedManufacturer(format!(
                        "MFR public key missing from cache for manufacturer={manufacturer}; call Hyper::verify_package first"
                    ))
                })
            }
        }
    }

    /// In production, prefetch the MFR public key over async HTTP and store it in `cert_cache`.
    pub async fn prefetch_mfr_cert(
        &self,
        manufacturer: &str,
        key_id: Option<&str>,
    ) -> HyperResult<()> {
        if let Some(cache) = &self.cert_cache {
            cache.get_or_fetch(manufacturer, key_id).await?;
        }
        Ok(())
    }
}

/// Detect `.actr` ZIP package by ZIP magic bytes (PK\x03\x04).
#[cfg(not(target_arch = "wasm32"))]
fn is_actr_package(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04"
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
