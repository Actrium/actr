use std::io::{Cursor, Read};

use crate::error::PackError;
use crate::manifest::PackageManifest;

/// Read the manifest from an .actr package without full verification.
pub fn read_manifest(actr_bytes: &[u8]) -> Result<PackageManifest, PackError> {
    let cursor = Cursor::new(actr_bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    let manifest_bytes =
        read_zip_entry(&mut archive, "actr.toml").map_err(|_| PackError::ManifestNotFound)?;

    let manifest_str = std::str::from_utf8(&manifest_bytes)
        .map_err(|e| PackError::ManifestParseError(format!("manifest is not valid UTF-8: {e}")))?;

    PackageManifest::from_toml(manifest_str)
}

/// Load the binary bytes from an .actr package.
///
/// Reads the manifest to determine the binary path, then extracts the binary.
pub fn load_binary(actr_bytes: &[u8]) -> Result<Vec<u8>, PackError> {
    let cursor = Cursor::new(actr_bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    // Read manifest to get binary path
    let manifest_bytes =
        read_zip_entry(&mut archive, "actr.toml").map_err(|_| PackError::ManifestNotFound)?;
    let manifest_str = std::str::from_utf8(&manifest_bytes)
        .map_err(|e| PackError::ManifestParseError(format!("manifest is not valid UTF-8: {e}")))?;
    let manifest = PackageManifest::from_toml(manifest_str)?;

    read_zip_entry(&mut archive, &manifest.binary.path)
        .map_err(|_| PackError::BinaryNotFound(manifest.binary.path.clone()))
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
