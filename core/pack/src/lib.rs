//! actr-pack -- .actr package format
//!
//! Provides reading, writing, signing and verification of .actr ZIP STORE packages.
//!
//! ## Package structure
//!
//! ```text
//! {mfr}-{name}-{version}-{target}.actr
//! +-- manifest.toml       # manifest (TOML, signed payload)
//! +-- manifest.sig        # Ed25519 signature (64 bytes raw)
//! +-- manifest.lock.toml  # dependency lock (optional)
//! +-- bin/actor.wasm      # binary (STORE mode, uncompressed)
//! +-- proto/*.proto       # exported proto files (optional)
//! ```
//!
//! ## Signing chain
//!
//! ```text
//! binary bytes -> SHA-256 -> manifest.toml[binary.hash]
//!                                    |
//!                          manifest.toml bytes -> Ed25519 sign -> manifest.sig
//! ```

pub mod error;
pub mod load;
pub mod manifest;
pub mod pack;
pub mod verify;

mod util;

pub use error::PackError;
pub use load::{
    load_binary, read_lock_file, read_manifest, read_manifest_raw, read_proto_files, read_signature,
};
pub use manifest::{
    BinaryEntry, LockFileEntry, ManifestMetadata, PackageManifest, ProtoFileEntry, ResourceEntry,
};
pub use pack::{PackOptions, pack};
pub use verify::{VerifiedPackage, verify};

/// Compute deterministic key_id from Ed25519 public key bytes.
///
/// Algorithm: `"mfr-" + hex(sha256(public_key_bytes))[..16]`
///
/// This MUST match the server-side implementation in `actrix-mfr::crypto::compute_key_id`.
pub fn compute_key_id(public_key_bytes: &[u8]) -> String {
    let hex_str = util::sha256_hex(public_key_bytes);
    format!("mfr-{}", &hex_str[..16])
}

/// Calculate ServiceSpec from a .actr package by extracting proto files and computing fingerprints.
///
/// This function:
/// 1. Reads proto file contents from the ZIP archive
/// 2. Calculates service-level semantic fingerprint
/// 3. Calculates per-file fingerprints
/// 4. Constructs a ServiceSpec for AIS/signaling registration
///
/// Returns `None` if the package contains no proto files.
pub fn calculate_service_spec_from_package(
    package_bytes: &[u8],
    manifest: &PackageManifest,
) -> Result<Option<actr_protocol::ServiceSpec>, PackError> {
    use std::io::Read;

    if manifest.proto_files.is_empty() {
        return Ok(None);
    }

    // 1. Extract proto file contents from ZIP
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(package_bytes))
        .map_err(|e| PackError::InvalidPackage(format!("Failed to open ZIP archive: {}", e)))?;

    let mut proto_contents = Vec::new();

    for proto_entry in &manifest.proto_files {
        let mut file = archive.by_name(&proto_entry.path).map_err(|e| {
            PackError::InvalidPackage(format!("Proto file not found in ZIP: {}", e))
        })?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| PackError::InvalidPackage(format!("Failed to read proto file: {}", e)))?;

        proto_contents.push((proto_entry.name.clone(), content));
    }

    // 2. Calculate service-level semantic fingerprint
    let proto_files: Vec<actr_service_compat::ProtoFile> = proto_contents
        .iter()
        .map(|(name, content)| actr_service_compat::ProtoFile {
            name: name.clone(),
            content: content.clone(),
            path: None,
        })
        .collect();

    let fingerprint =
        actr_service_compat::Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
            .map_err(|e| {
                PackError::InvalidPackage(format!("Failed to calculate fingerprint: {}", e))
            })?;

    // 3. Construct ServiceSpec with per-file fingerprints
    let protobufs = proto_contents
        .iter()
        .map(|(name, content)| {
            let file_fingerprint =
                actr_service_compat::Fingerprint::calculate_proto_semantic_fingerprint(content)
                    .unwrap_or_else(|_| "error".to_string());

            actr_protocol::service_spec::Protobuf {
                package: name.trim_end_matches(".proto").to_string(),
                content: content.clone(),
                fingerprint: file_fingerprint,
            }
        })
        .collect();

    let published_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64);

    Ok(Some(actr_protocol::ServiceSpec {
        name: manifest.name.clone(),
        description: manifest.metadata.description.clone(),
        fingerprint,
        protobufs,
        published_at,
        tags: vec![],
    }))
}
