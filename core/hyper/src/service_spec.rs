//! Derive [`actr_protocol::ServiceSpec`] from a verified `.actr` package.
//!
//! Moved out of `actr-pack` because it pulls in `actr-service-compat` (and
//! therefore `proto-fingerprint`), which is not wasm-friendly. `actr-pack`
//! stays wasm-compatible for the SW runtime; the spec derivation lives here
//! alongside the only caller (`Node<Attached>::register`).
//!
//! The actual spec assembly (fingerprint computation, `Protobuf` packing)
//! lives in [`actr_service_compat::build_service_spec`]; this module is just
//! the ZIP-extraction adapter.

use actr_pack::PackageManifest;
use actr_protocol::ServiceSpec;
use actr_service_compat::{ProtoFile, ServiceSpecInput, build_service_spec};
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

    let mut proto_files = Vec::with_capacity(manifest.proto_files.len());
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
        proto_files.push(ProtoFile {
            name: proto_entry.name.clone(),
            content,
            path: None,
        });
    }

    let spec = build_service_spec(ServiceSpecInput {
        name: &manifest.name,
        description: manifest.metadata.description.clone(),
        tags: vec![],
        proto_files,
    })
    .map_err(|e| HyperError::Runtime(format!("build service spec: {e}")))?;

    Ok(Some(spec))
}
