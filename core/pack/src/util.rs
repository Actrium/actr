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

/// Compute a lowercase hex-encoded SHA-256 digest of `data`.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
