//! Derive [`actr_protocol::ServiceSpec`] from a verified `.actr` package.
//!
//! Moved out of `actr-pack` because it pulls in `actr-service-compat` (and
//! therefore `proto-fingerprint`), which is not wasm-friendly. `actr-pack`
//! stays wasm-compatible for the SW runtime; the spec derivation lives here
//! alongside the only caller (`Hyper<Attached>::register`).

use actr_pack::PackageManifest;
use actr_protocol::ServiceSpec;
use std::io::Read;

use crate::error::{HyperError, HyperResult};

/// Build a [`ServiceSpec`] from the package's proto exports.
///
/// Returns `Ok(None)` when the manifest declares no proto files — such
/// packages register with AIS without a service spec.
pub(crate) fn calculate_service_spec_from_package(
    package_bytes: &[u8],
    manifest: &PackageManifest,
) -> HyperResult<Option<ServiceSpec>> {
    if manifest.proto_files.is_empty() {
        return Ok(None);
    }

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(package_bytes))
        .map_err(|e| HyperError::Runtime(format!("open package ZIP: {e}")))?;

    let mut proto_contents = Vec::with_capacity(manifest.proto_files.len());
    for proto_entry in &manifest.proto_files {
        let mut file = archive.by_name(&proto_entry.path).map_err(|e| {
            HyperError::Runtime(format!(
                "proto file {} not found in package: {e}",
                proto_entry.path
            ))
        })?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| HyperError::Runtime(format!("read proto {}: {e}", proto_entry.path)))?;
        proto_contents.push((proto_entry.name.clone(), content));
    }

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
                HyperError::Runtime(format!("calculate service fingerprint: {e}"))
            })?;

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

    Ok(Some(ServiceSpec {
        name: manifest.name.clone(),
        description: manifest.metadata.description.clone(),
        fingerprint,
        protobufs,
        published_at,
        tags: vec![],
    }))
}
