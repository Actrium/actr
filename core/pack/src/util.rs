//! Internal helpers shared between load/verify/pack modules.

use std::io::{Read, Seek};

use sha2::{Digest, Sha256};

use crate::error::PackError;

/// Read a ZIP entry fully into a byte vector, preallocating based on the entry size.
pub(crate) fn read_zip_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, PackError> {
    let mut entry = archive.by_name(name)?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Read a ZIP entry without allowing its declared or actual decompressed size
/// to exceed `max_bytes`.
pub(crate) fn read_zip_entry_bounded<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, PackError> {
    let entry = archive.by_name(name)?;
    if entry.size() > max_bytes as u64 {
        return Err(PackError::InvalidPackage(format!(
            "ZIP entry `{name}` declares {} bytes, exceeds limit {max_bytes}",
            entry.size()
        )));
    }
    let capacity = usize::try_from(entry.size())
        .unwrap_or(max_bytes)
        .min(max_bytes);
    let mut buf = Vec::with_capacity(capacity);
    let mut limited = entry.take(max_bytes as u64 + 1);
    limited.read_to_end(&mut buf)?;
    if buf.len() > max_bytes {
        return Err(PackError::InvalidPackage(format!(
            "ZIP entry `{name}` exceeds decompressed limit {max_bytes}"
        )));
    }
    Ok(buf)
}

/// Compute a lowercase hex-encoded SHA-256 digest of `data`.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
