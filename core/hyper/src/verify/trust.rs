//! Trust provider — pluggable verifier for `.actr` package signatures.
//!
//! Replaces the old `TrustMode` enum. A `TrustProvider` answers the only
//! question Hyper cares about at load time: "is this package bytes-authentic
//! enough for me to execute?". How it answers is up to the provider:
//!
//! - [`StaticTrust`] — one pre-configured Ed25519 public key, accepts any
//!   manufacturer. Offline / air-gapped deployments.
//! - [`RegistryTrust`] — fetch MFR public keys from an AIS HTTP registry by
//!   `(manufacturer, signing_key_id)`, cached locally.
//! - [`ChainTrust`] — try a list of providers in order; first success wins.
//!
//! Both built-in Ed25519-based providers delegate to [`actr_pack::verify`],
//! which performs the full signature + binary-hash + resource-hash chain.
//! Custom providers (e.g. wasm-side keyless verification, HSM, threshold
//! signatures) may implement [`TrustProvider`] however they want — the trait
//! only obliges them to take raw bytes in and return a verified manifest out.

use std::sync::Arc;

use async_trait::async_trait;
use ed25519_dalek::VerifyingKey;

use crate::error::{HyperError, HyperResult};
use crate::verify::cert_cache::MfrCertCache;
use crate::verify::manifest::PackageManifest;

/// Verifier for `.actr` package signatures.
///
/// An implementation fully takes raw package bytes and returns the parsed,
/// trusted manifest — or errors. Callers must not use any field of the
/// returned [`PackageManifest`] before calling this.
#[async_trait]
pub trait TrustProvider: Send + Sync + std::fmt::Debug {
    async fn verify_package(&self, bytes: &[u8]) -> HyperResult<PackageManifest>;
}

// ── shared helper for the Ed25519 + pubkey path ──────────────────────────────

/// Verify an `.actr` package against a single Ed25519 public key.
///
/// Shared helper used by [`StaticTrust`] and [`RegistryTrust`]; also available
/// to custom providers that want the standard signature + hash chain.
pub fn verify_ed25519_manifest(
    bytes: &[u8],
    pubkey: &VerifyingKey,
) -> HyperResult<PackageManifest> {
    let verified = actr_pack::verify(bytes, pubkey).map_err(|e| match e {
        actr_pack::PackError::SignatureVerificationFailed(msg) => {
            HyperError::SignatureVerificationFailed(msg)
        }
        actr_pack::PackError::BinaryHashMismatch { .. } => HyperError::BinaryHashMismatch,
        actr_pack::PackError::SignatureNotFound => {
            HyperError::SignatureVerificationFailed("signature not found in package".to_string())
        }
        actr_pack::PackError::BinaryNotFound(path) => {
            HyperError::InvalidManifest(format!("binary not found: {path}"))
        }
        actr_pack::PackError::ManifestNotFound => HyperError::ManifestNotFound,
        actr_pack::PackError::ManifestParseError(msg) => HyperError::InvalidManifest(msg),
        other => HyperError::InvalidManifest(other.to_string()),
    })?;

    tracing::info!(
        actr_type = %verified.manifest.actr_type_str(),
        ".actr package verified"
    );

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

fn parse_pubkey(bytes: &[u8]) -> HyperResult<VerifyingKey> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| HyperError::Config("Ed25519 pubkey must be exactly 32 bytes".to_string()))?;
    VerifyingKey::from_bytes(&arr)
        .map_err(|e| HyperError::Config(format!("invalid Ed25519 pubkey: {e}")))
}

// ── StaticTrust ──────────────────────────────────────────────────────────────

/// Pre-configured single Ed25519 public key. Accepts packages from any
/// manufacturer as long as they verify against this key.
///
/// Intended for dev / air-gapped / self-hosted deployments where the
/// manufacturer's public key is shipped alongside the package (typically as
/// `public-key.json`) instead of queried from a registry.
#[derive(Debug, Clone)]
pub struct StaticTrust {
    pubkey: VerifyingKey,
}

impl StaticTrust {
    /// Construct from 32 raw Ed25519 public key bytes.
    pub fn new(pubkey: impl AsRef<[u8]>) -> HyperResult<Self> {
        Ok(Self {
            pubkey: parse_pubkey(pubkey.as_ref())?,
        })
    }
}

#[async_trait]
impl TrustProvider for StaticTrust {
    async fn verify_package(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        verify_ed25519_manifest(bytes, &self.pubkey)
    }
}

// ── RegistryTrust ────────────────────────────────────────────────────────────

/// Resolve manufacturer public keys from an AIS HTTP registry and verify
/// Ed25519 signatures against them. Internal cache with configurable TTL
/// (default 1h).
///
/// The package manifest must carry `signing_key_id`; otherwise the provider
/// errors out — rebuild with the latest `actr build` to embed one.
#[derive(Debug, Clone)]
pub struct RegistryTrust {
    cache: Arc<MfrCertCache>,
}

impl RegistryTrust {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            cache: MfrCertCache::new(endpoint),
        }
    }
}

#[async_trait]
impl TrustProvider for RegistryTrust {
    async fn verify_package(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        let pack_manifest = actr_pack::read_manifest(bytes).map_err(|e| match e {
            actr_pack::PackError::ManifestNotFound => HyperError::ManifestNotFound,
            actr_pack::PackError::ManifestParseError(msg) => HyperError::InvalidManifest(msg),
            other => HyperError::InvalidManifest(other.to_string()),
        })?;

        let key_id = pack_manifest.signing_key_id.as_deref().ok_or_else(|| {
            HyperError::InvalidManifest(
                "package manifest missing `signing_key_id`; rebuild with the latest `actr build`"
                    .to_string(),
            )
        })?;

        let pubkey = self
            .cache
            .get_or_fetch(&pack_manifest.manufacturer, Some(key_id))
            .await?;

        verify_ed25519_manifest(bytes, &pubkey)
    }
}

// ── ChainTrust ───────────────────────────────────────────────────────────────

/// Try a list of providers in order; the first `Ok(_)` wins.
///
/// Useful for "local cache first, registry fallback" setups or for rolling
/// key migrations where an old static key and a new registry-backed provider
/// coexist.
#[derive(Debug, Clone)]
pub struct ChainTrust {
    providers: Vec<Arc<dyn TrustProvider>>,
}

impl ChainTrust {
    pub fn new(providers: Vec<Arc<dyn TrustProvider>>) -> Self {
        Self { providers }
    }

    /// Shortcut for a two-provider chain.
    pub fn of(first: Arc<dyn TrustProvider>, second: Arc<dyn TrustProvider>) -> Self {
        Self::new(vec![first, second])
    }
}

#[async_trait]
impl TrustProvider for ChainTrust {
    async fn verify_package(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        let mut last_err: Option<HyperError> = None;
        for p in &self.providers {
            match p.verify_package(bytes).await {
                Ok(m) => return Ok(m),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            HyperError::SignatureVerificationFailed("empty trust chain".to_string())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn make_minimal_package(signing_key: &SigningKey) -> Vec<u8> {
        let manifest = actr_pack::PackageManifest {
            manufacturer: "test-mfr".to_string(),
            name: "Test".to_string(),
            version: "1.0.0".to_string(),
            binary: actr_pack::BinaryEntry {
                path: "bin/actor.wasm".to_string(),
                target: "wasm32-wasip1".to_string(),
                hash: String::new(),
                size: None,
            },
            signature_algorithm: "ed25519".to_string(),
            signing_key_id: Some(actr_pack::compute_key_id(
                &signing_key.verifying_key().to_bytes(),
            )),
            resources: vec![],
            proto_files: vec![],
            lock_file: None,
            metadata: actr_pack::ManifestMetadata::default(),
        };
        actr_pack::pack(&actr_pack::PackOptions {
            manifest,
            binary_bytes: b"wasm".to_vec(),
            resources: vec![],
            proto_files: vec![],
            lock_file: None,
            signing_key: signing_key.clone(),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn static_trust_accepts_valid_package() {
        let key = SigningKey::generate(&mut OsRng);
        let vk = key.verifying_key();
        let pkg = make_minimal_package(&key);

        let trust = StaticTrust::new(vk.to_bytes()).unwrap();
        let m = trust.verify_package(&pkg).await.unwrap();
        assert_eq!(m.manufacturer, "test-mfr");
    }

    #[tokio::test]
    async fn static_trust_rejects_wrong_key() {
        let key = SigningKey::generate(&mut OsRng);
        let wrong = SigningKey::generate(&mut OsRng);
        let pkg = make_minimal_package(&key);

        let trust = StaticTrust::new(wrong.verifying_key().to_bytes()).unwrap();
        assert!(matches!(
            trust.verify_package(&pkg).await,
            Err(HyperError::SignatureVerificationFailed(_))
        ));
    }

    #[tokio::test]
    async fn chain_first_match_wins() {
        let key = SigningKey::generate(&mut OsRng);
        let other = SigningKey::generate(&mut OsRng);
        let pkg = make_minimal_package(&key);

        let wrong: Arc<dyn TrustProvider> =
            Arc::new(StaticTrust::new(other.verifying_key().to_bytes()).unwrap());
        let right: Arc<dyn TrustProvider> =
            Arc::new(StaticTrust::new(key.verifying_key().to_bytes()).unwrap());

        let chain = ChainTrust::of(wrong, right);
        let m = chain.verify_package(&pkg).await.unwrap();
        assert_eq!(m.manufacturer, "test-mfr");
    }

    #[tokio::test]
    async fn chain_all_fail_returns_last_error() {
        let key = SigningKey::generate(&mut OsRng);
        let wrong1 = SigningKey::generate(&mut OsRng);
        let wrong2 = SigningKey::generate(&mut OsRng);
        let pkg = make_minimal_package(&key);

        let chain = ChainTrust::of(
            Arc::new(StaticTrust::new(wrong1.verifying_key().to_bytes()).unwrap()),
            Arc::new(StaticTrust::new(wrong2.verifying_key().to_bytes()).unwrap()),
        );
        assert!(matches!(
            chain.verify_package(&pkg).await,
            Err(HyperError::SignatureVerificationFailed(_))
        ));
    }

    // Just so the minimum-bound test doesn't compile away unused Signer import.
    #[allow(dead_code)]
    fn _signer_sanity(key: &SigningKey) -> ed25519_dalek::Signature {
        key.sign(b"x")
    }
}
