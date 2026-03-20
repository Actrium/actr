use std::io::{Cursor, Read};

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::error::PackError;
use crate::manifest::PackageManifest;

/// Result of a successful package verification.
///
/// Contains the parsed manifest along with the raw bytes needed for
/// transparent forwarding to AIS for signature verification.
#[derive(Debug)]
pub struct VerifiedPackage {
    /// Parsed package manifest.
    pub manifest: PackageManifest,
    /// Raw `actr.toml` bytes as stored in the ZIP (the signed payload).
    pub manifest_raw: Vec<u8>,
    /// Raw `actr.sig` bytes (64-byte Ed25519 signature).
    pub sig_raw: Vec<u8>,
}

/// Verify an .actr package.
///
/// Verification flow:
/// 1. Read actr.sig (64 bytes raw Ed25519 signature)
/// 2. Read actr.toml (raw bytes)
/// 3. Verify Ed25519 signature over actr.toml bytes
/// 4. Parse actr.toml -> PackageManifest
/// 5. Read binary, verify SHA-256 matches manifest.binary.hash
/// 6. For each resource, verify SHA-256 matches entry hash
/// 7. Return VerifiedPackage with manifest + raw bytes
pub fn verify(actr_bytes: &[u8], pubkey: &VerifyingKey) -> Result<VerifiedPackage, PackError> {
    let cursor = Cursor::new(actr_bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    // 1. Read actr.sig
    let sig_raw =
        read_zip_entry(&mut archive, "actr.sig").map_err(|_| PackError::SignatureNotFound)?;
    if sig_raw.len() != 64 {
        return Err(PackError::SignatureVerificationFailed(format!(
            "actr.sig must be exactly 64 bytes, got {}",
            sig_raw.len()
        )));
    }
    let sig_arr: [u8; 64] = sig_raw.clone().try_into().unwrap();
    let signature = Signature::from_bytes(&sig_arr);

    // 2. Read actr.toml
    let manifest_bytes =
        read_zip_entry(&mut archive, "actr.toml").map_err(|_| PackError::ManifestNotFound)?;

    // 3. Verify signature over actr.toml
    pubkey
        .verify_strict(&manifest_bytes, &signature)
        .map_err(|e| {
            PackError::SignatureVerificationFailed(format!("Ed25519 verification failed: {e}"))
        })?;

    tracing::debug!("package signature verified");

    // 4. Parse manifest
    let manifest_str = std::str::from_utf8(&manifest_bytes)
        .map_err(|e| PackError::ManifestParseError(format!("manifest is not valid UTF-8: {e}")))?;
    let manifest = PackageManifest::from_toml(manifest_str)?;

    // 5. Verify binary hash
    let binary_bytes = read_zip_entry(&mut archive, &manifest.binary.path)
        .map_err(|_| PackError::BinaryNotFound(manifest.binary.path.clone()))?;
    let computed_hash = sha256_hex(&binary_bytes);
    if computed_hash != manifest.binary.hash {
        tracing::warn!(
            expected = %manifest.binary.hash,
            computed = %computed_hash,
            path = %manifest.binary.path,
            "binary hash mismatch"
        );
        return Err(PackError::BinaryHashMismatch {
            path: manifest.binary.path.clone(),
        });
    }

    // 6. Verify resource hashes
    for resource in &manifest.resources {
        let res_bytes = read_zip_entry(&mut archive, &resource.path)
            .map_err(|_| PackError::BinaryNotFound(resource.path.clone()))?;
        let computed = sha256_hex(&res_bytes);
        if computed != resource.hash {
            tracing::warn!(
                expected = %resource.hash,
                computed = %computed,
                path = %resource.path,
                "resource hash mismatch"
            );
            return Err(PackError::ResourceHashMismatch {
                path: resource.path.clone(),
            });
        }
    }

    tracing::info!(
        actr_type = %manifest.actr_type_str(),
        "package verification passed"
    );

    Ok(VerifiedPackage {
        manifest,
        manifest_raw: manifest_bytes,
        sig_raw,
    })
}

fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, PackError> {
    let mut entry = archive.by_name(name)?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BinaryEntry, ManifestMetadata, PackageManifest, ResourceEntry};
    use crate::pack::{PackOptions, pack};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use std::io::Write;

    fn test_manifest() -> PackageManifest {
        PackageManifest {
            manufacturer: "test-mfr".to_string(),
            name: "TestActor".to_string(),
            version: "1.0.0".to_string(),
            binary: BinaryEntry {
                path: "bin/actor.wasm".to_string(),
                target: "wasm32-wasip1".to_string(),
                hash: String::new(),
                size: None,
            },
            signature_algorithm: "ed25519".to_string(),
            resources: vec![],
            metadata: ManifestMetadata::default(),
        }
    }

    fn make_package(
        signing_key: &SigningKey,
        binary: &[u8],
        resources: Vec<(String, Vec<u8>)>,
    ) -> Vec<u8> {
        let mut manifest = test_manifest();
        manifest.resources = resources
            .iter()
            .map(|(path, _)| ResourceEntry {
                path: path.clone(),
                hash: String::new(),
            })
            .collect();
        let opts = PackOptions {
            manifest,
            binary_bytes: binary.to_vec(),
            resources,
            signing_key: signing_key.clone(),
        };
        pack(&opts).unwrap()
    }

    #[test]
    fn roundtrip_succeeds() {
        let key = SigningKey::generate(&mut OsRng);
        let pkg = make_package(&key, b"wasm bytes", vec![]);
        let result = verify(&pkg, &key.verifying_key()).unwrap();
        assert_eq!(result.manifest.manufacturer, "test-mfr");
        assert_eq!(result.manifest.name, "TestActor");
        assert_eq!(result.sig_raw.len(), 64);
        assert!(!result.manifest_raw.is_empty());
    }

    #[test]
    fn tampered_binary_detected() {
        let key = SigningKey::generate(&mut OsRng);
        let pkg_bytes = make_package(&key, b"original", vec![]);
        // Modify a byte deep in the file (in the binary data area)
        let mut tampered = pkg_bytes.clone();
        // Find "original" in the ZIP and change it
        if let Some(pos) = tampered.windows(8).position(|w| w == b"original") {
            tampered[pos] ^= 0xFF;
        }
        let result = verify(&tampered, &key.verifying_key());
        // Should fail with either signature or hash mismatch
        assert!(
            result.is_err(),
            "tampered package should fail: {:?}",
            result
        );
    }

    #[test]
    fn wrong_key_rejected() {
        let key1 = SigningKey::generate(&mut OsRng);
        let key2 = SigningKey::generate(&mut OsRng);
        let pkg = make_package(&key1, b"wasm", vec![]);
        let result = verify(&pkg, &key2.verifying_key());
        assert!(matches!(
            result,
            Err(PackError::SignatureVerificationFailed(_))
        ));
    }

    #[test]
    fn missing_signature_detected() {
        // Create a ZIP without actr.sig
        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("actr.toml", opts).unwrap();
        zip.write_all(b"[fake]").unwrap();
        let data = zip.finish().unwrap().into_inner();

        let key = SigningKey::generate(&mut OsRng);
        let result = verify(&data, &key.verifying_key());
        assert!(matches!(result, Err(PackError::SignatureNotFound)));
    }

    #[test]
    fn resource_hash_mismatch_detected() {
        let key = SigningKey::generate(&mut OsRng);
        let pkg = make_package(
            &key,
            b"wasm",
            vec![(
                "config/settings.toml".to_string(),
                b"key = \"value\"".to_vec(),
            )],
        );
        // Tamper the resource
        let mut tampered = pkg.clone();
        if let Some(pos) = tampered.windows(5).position(|w| w == b"value") {
            tampered[pos] ^= 0xFF;
        }
        let result = verify(&tampered, &key.verifying_key());
        assert!(result.is_err());
    }

    #[test]
    fn with_resources_roundtrip() {
        let key = SigningKey::generate(&mut OsRng);
        let resources = vec![
            ("config/a.toml".to_string(), b"data_a".to_vec()),
            ("config/b.toml".to_string(), b"data_b".to_vec()),
        ];
        let pkg = make_package(&key, b"wasm", resources);
        let result = verify(&pkg, &key.verifying_key()).unwrap();
        assert_eq!(result.manifest.resources.len(), 2);
    }
}
