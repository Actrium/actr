use std::io::{Cursor, Write};

use ed25519_dalek::{Signer, SigningKey};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::error::PackError;
use crate::manifest::PackageManifest;
use crate::util::sha256_hex;

/// Options for creating an .actr package.
pub struct PackOptions {
    /// Manifest template (binary.hash/size will be computed and filled)
    pub manifest: PackageManifest,
    /// Binary bytes (the actor wasm/native binary)
    pub binary_bytes: Vec<u8>,
    /// Resources: (path, bytes) pairs
    pub resources: Vec<(String, Vec<u8>)>,
    /// Proto files: (filename, content) pairs.
    /// Written to `proto/` directory inside the ZIP.
    pub proto_files: Vec<(String, Vec<u8>)>,
    /// Ed25519 signing key
    pub signing_key: SigningKey,
    /// Optional manifest.lock.toml content — packed as `manifest.lock.toml` in the ZIP
    pub lock_file: Option<Vec<u8>>,
}

/// Create an .actr package (ZIP STORE format).
///
/// Returns the complete package bytes.
pub fn pack(opts: &PackOptions) -> Result<Vec<u8>, PackError> {
    let mut manifest = opts.manifest.clone();

    // 1. Compute binary SHA-256 hash
    let binary_hash = sha256_hex(&opts.binary_bytes);
    manifest.binary.hash = binary_hash;
    manifest.binary.size = Some(opts.binary_bytes.len() as u64);

    // 2. Compute resource hashes
    if manifest.resources.len() != opts.resources.len() {
        // Rebuild resources from provided data
        manifest.resources = opts
            .resources
            .iter()
            .map(|(path, bytes)| crate::manifest::ResourceEntry {
                path: path.clone(),
                hash: sha256_hex(bytes),
            })
            .collect();
    } else {
        for (i, (_path, bytes)) in opts.resources.iter().enumerate() {
            manifest.resources[i].hash = sha256_hex(bytes);
        }
    }

    // 2.5. Compute proto file hashes and build entries
    manifest.proto_files = opts
        .proto_files
        .iter()
        .map(|(name, content)| crate::manifest::ProtoFileEntry {
            name: name.clone(),
            path: format!("proto/{}", name),
            hash: sha256_hex(content),
        })
        .collect();

    manifest.lock_file = opts
        .lock_file
        .as_ref()
        .map(|bytes| crate::manifest::LockFileEntry {
            path: "manifest.lock.toml".to_string(),
            hash: sha256_hex(bytes),
        });

    // 3. Serialize manifest to TOML
    let manifest_toml = manifest.to_toml()?;
    let manifest_bytes = manifest_toml.as_bytes();

    // 4. Sign the manifest TOML bytes
    let signature = opts.signing_key.sign(manifest_bytes);
    let sig_bytes = signature.to_bytes();

    tracing::info!(
        actr_type = %manifest.actr_type_str(),
        binary_path = %manifest.binary.path,
        binary_size = opts.binary_bytes.len(),
        resources = opts.resources.len(),
        "packing .actr file"
    );

    // 5. Write ZIP (STORE mode)
    let buf = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(buf);
    let store_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    // manifest.toml (the signed payload)
    zip.start_file("manifest.toml", store_opts)?;
    zip.write_all(manifest_bytes)?;

    // manifest.sig (64 bytes raw Ed25519)
    zip.start_file("manifest.sig", store_opts)?;
    zip.write_all(&sig_bytes)?;

    // manifest.lock.toml (dependency lock, optional)
    if let Some(lock_bytes) = &opts.lock_file {
        zip.start_file("manifest.lock.toml", store_opts)?;
        zip.write_all(lock_bytes)?;
    }

    // binary
    zip.start_file(&manifest.binary.path, store_opts)?;
    zip.write_all(&opts.binary_bytes)?;

    // resources
    for (path, bytes) in &opts.resources {
        zip.start_file(path.as_str(), store_opts)?;
        zip.write_all(bytes)?;
    }

    // proto files
    for (name, content) in &opts.proto_files {
        let zip_path = format!("proto/{}", name);
        zip.start_file(&zip_path, store_opts)?;
        zip.write_all(content)?;
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BinaryEntry, ManifestMetadata, PackageManifest};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn test_manifest() -> PackageManifest {
        PackageManifest {
            manufacturer: "test-mfr".to_string(),
            name: "TestActor".to_string(),
            version: "1.0.0".to_string(),
            binary: BinaryEntry {
                path: "bin/actor.wasm".to_string(),
                target: "wasm32-wasip1".to_string(),
                hash: String::new(), // will be computed
                size: None,
                kind: None,
            },
            signature_algorithm: "ed25519".to_string(),
            signing_key_id: None,
            resources: vec![],
            proto_files: vec![],
            lock_file: None,
            metadata: ManifestMetadata::default(),
        }
    }

    #[test]
    fn pack_creates_valid_zip() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let opts = PackOptions {
            manifest: test_manifest(),
            binary_bytes: b"fake wasm binary".to_vec(),
            resources: vec![],
            proto_files: vec![],
            signing_key,
            lock_file: None,
        };
        let result = pack(&opts);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        // ZIP magic: PK\x03\x04
        assert_eq!(&bytes[0..2], b"PK");
    }

    #[test]
    fn pack_then_verify_roundtrip() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let opts = PackOptions {
            manifest: test_manifest(),
            binary_bytes: b"hello wasm".to_vec(),
            resources: vec![],
            proto_files: vec![],
            signing_key: signing_key.clone(),
            lock_file: None,
        };
        let package = pack(&opts).unwrap();
        let result = crate::verify::verify(&package, &verifying_key).unwrap();
        assert_eq!(result.manifest.manufacturer, "test-mfr");
        assert_eq!(result.manifest.name, "TestActor");
        assert_eq!(result.manifest.version, "1.0.0");
    }
}
