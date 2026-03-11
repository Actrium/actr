use sha2::{Digest, Sha256};

use crate::error::{HyperError, HyperResult};

/// Signed manifest embedded in an Actr package.
///
/// Stored in a binary custom section such as a WASM custom section, ELF section,
/// or Mach-O segment. The MFR signs it at build time and Hyper verifies it before loading.
#[derive(Debug, Clone)]
pub struct PackageManifest {
    /// Actor manufacturer name, the `manufacturer` component of `manufacturer:name:version`.
    pub manufacturer: String,
    /// Actor name.
    pub actr_name: String,
    /// Actor version.
    pub version: String,
    /// SHA-256 hash of the file with the signature section removed, 32 bytes.
    pub binary_hash: [u8; 32],
    /// Capability list declared by the actor.
    pub capabilities: Vec<String>,
    /// Ed25519 signature over the manifest contents, excluding the `signature` field itself.
    pub signature: Vec<u8>,
}

impl PackageManifest {
    /// Full three-part `ActrType` string.
    pub fn actr_type_str(&self) -> String {
        format!("{}:{}:{}", self.manufacturer, self.actr_name, self.version)
    }
}

// ─── WASM custom section parsing ────────────────────────────────────────────

/// WASM custom section name used to embed the manifest.
pub(crate) const WASM_MANIFEST_SECTION: &str = "actr-manifest";

/// Extract the manifest section payload from WASM bytes.
///
/// Returns the section payload slice without the section header, or `None` if not found.
pub fn extract_wasm_manifest(wasm_bytes: &[u8]) -> Option<&[u8]> {
    // WASM magic plus version occupies 8 bytes.
    if wasm_bytes.len() < 8 {
        return None;
    }
    if &wasm_bytes[0..4] != b"\0asm" {
        return None;
    }

    let mut pos = 8usize;
    while pos < wasm_bytes.len() {
        if pos + 1 > wasm_bytes.len() {
            break;
        }
        let section_id = wasm_bytes[pos];
        pos += 1;

        // Read the LEB128-encoded section size.
        let (section_size, bytes_read) = read_leb128_u32(&wasm_bytes[pos..])?;
        pos += bytes_read;

        let section_end = pos + section_size as usize;
        if section_end > wasm_bytes.len() {
            break;
        }

        // `section_id == 0` indicates a custom section.
        if section_id == 0 {
            // A custom section starts with name length plus name bytes.
            let (name_len, name_bytes_read) = read_leb128_u32(&wasm_bytes[pos..])?;
            let name_start = pos + name_bytes_read;
            let name_end = name_start + name_len as usize;

            if name_end <= section_end {
                if let Ok(name) = std::str::from_utf8(&wasm_bytes[name_start..name_end]) {
                    if name == WASM_MANIFEST_SECTION {
                        // The payload starts after the section name.
                        return Some(&wasm_bytes[name_end..section_end]);
                    }
                }
            }
        }

        pos = section_end;
    }

    None
}

/// Compute the SHA-256 hash of a WASM file with the manifest custom section removed.
///
/// This is how `binary_hash` is computed: hash before writing the section during signing,
/// and remove the section before recomputing during verification to avoid circular dependence.
pub fn wasm_binary_hash_excluding_manifest(wasm_bytes: &[u8]) -> HyperResult<[u8; 32]> {
    if wasm_bytes.len() < 8 || &wasm_bytes[0..4] != b"\0asm" {
        return Err(HyperError::InvalidManifest(
            "Not a valid WASM file".to_string(),
        ));
    }

    let mut hasher = Sha256::new();
    // Write magic plus version.
    hasher.update(&wasm_bytes[0..8]);

    let mut pos = 8usize;
    while pos < wasm_bytes.len() {
        if pos + 1 > wasm_bytes.len() {
            break;
        }
        let section_id_pos = pos;
        let section_id = wasm_bytes[pos];
        pos += 1;

        let (section_size, bytes_read) = read_leb128_u32(&wasm_bytes[pos..]).ok_or_else(|| {
            HyperError::InvalidManifest("Failed to decode WASM section size as LEB128".to_string())
        })?;
        let header_end = pos + bytes_read;
        pos = header_end;

        let section_end = pos + section_size as usize;
        if section_end > wasm_bytes.len() {
            return Err(HyperError::InvalidManifest(
                "WASM section exceeds file bounds".to_string(),
            ));
        }

        // Skip the manifest custom section so it does not participate in the hash.
        if section_id == 0 {
            let (name_len, name_bytes_read) =
                read_leb128_u32(&wasm_bytes[pos..]).ok_or_else(|| {
                    HyperError::InvalidManifest(
                        "Failed to decode custom section name length".to_string(),
                    )
                })?;
            let name_start = pos + name_bytes_read;
            let name_end = name_start + name_len as usize;

            if name_end <= section_end {
                if let Ok(name) = std::str::from_utf8(&wasm_bytes[name_start..name_end]) {
                    if name == WASM_MANIFEST_SECTION {
                        // Skip this section and do not hash it.
                        pos = section_end;
                        continue;
                    }
                }
            }
        }

        // Include all other sections in the hash, including section id, size, and contents.
        hasher.update(&wasm_bytes[section_id_pos..section_end]);
        pos = section_end;
    }

    Ok(hasher.finalize().into())
}

pub fn is_wasm(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == b"\0asm"
}

// ─── ELF section parsing ────────────────────────────────────────────────────

/// ELF section name used to embed the manifest.
const ELF_MANIFEST_SECTION: &str = "actr_manifest";

/// Minimum size of an ELF64 section header in bytes.
const ELF64_SHDR_SIZE: usize = 64;

/// Extract the manifest section payload from ELF bytes.
///
/// Supports ELF64 little-endian only, such as x86_64 and aarch64.
/// Returns the payload slice of the `actr_manifest` section, or `None` when not found.
pub fn extract_elf_manifest(bytes: &[u8]) -> Option<&[u8]> {
    // ELF magic: `\x7fELF`. The ELF header is at least 64 bytes.
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return None;
    }
    // `EI_CLASS` at offset 4: 2 means ELF64.
    if bytes[4] != 2 {
        tracing::debug!("ELF file is 32-bit and is not supported yet");
        return None;
    }
    // `EI_DATA` at offset 5: 1 means little-endian.
    if bytes[5] != 1 {
        tracing::debug!("ELF file is big-endian and is not supported yet");
        return None;
    }

    let (sh_off, sh_size) = elf_manifest_data_range_inner(bytes)?;
    let data_end = sh_off.checked_add(sh_size)?;
    if data_end > bytes.len() {
        return None;
    }
    tracing::debug!(
        section_offset = sh_off,
        section_size = sh_size,
        "Found ELF actr_manifest section"
    );
    Some(&bytes[sh_off..data_end])
}

/// Compute the SHA-256 hash of an ELF file with the manifest section zero-filled.
///
/// Zero-fill means replacing the section data region with zero bytes before hashing,
/// while preserving file size and offsets to match the `binary_hash` signing contract.
pub fn elf_binary_hash_excluding_manifest(bytes: &[u8]) -> HyperResult<[u8; 32]> {
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return Err(HyperError::InvalidManifest(
            "Not a valid ELF file".to_string(),
        ));
    }
    if bytes[4] != 2 {
        return Err(HyperError::InvalidManifest(
            "Only ELF64 is supported".to_string(),
        ));
    }
    if bytes[5] != 1 {
        return Err(HyperError::InvalidManifest(
            "Only ELF little-endian format is supported".to_string(),
        ));
    }

    let manifest_range = elf_manifest_data_range(bytes)?;

    let mut hasher = Sha256::new();
    if let Some((data_off, data_size)) = manifest_range {
        // Zero-fill the manifest section data region; hash the rest normally.
        hasher.update(&bytes[..data_off]);
        hasher.update(vec![0u8; data_size]);
        hasher.update(&bytes[data_off + data_size..]);
    } else {
        // No manifest section was found, so hash the entire file.
        tracing::debug!("No actr_manifest section found in ELF; hashing the entire file");
        hasher.update(bytes);
    }

    Ok(hasher.finalize().into())
}

/// Return the `(file_offset, size)` pair for the ELF `actr_manifest` section, or an error.
pub fn elf_manifest_data_range(bytes: &[u8]) -> HyperResult<Option<(usize, usize)>> {
    Ok(elf_manifest_data_range_inner(bytes))
}

/// Internal `Option`-based lookup of the ELF `actr_manifest` data range.
fn elf_manifest_data_range_inner(bytes: &[u8]) -> Option<(usize, usize)> {
    // e_shoff (offset 40, 8 bytes): file offset of the section header table.
    let e_shoff = u64::from_le_bytes(bytes[40..48].try_into().ok()?) as usize;
    // e_shentsize (offset 58, 2 bytes): size of each section header.
    let e_shentsize = u16::from_le_bytes(bytes[58..60].try_into().ok()?) as usize;
    // e_shnum (offset 60, 2 bytes): number of section header entries.
    let e_shnum = u16::from_le_bytes(bytes[60..62].try_into().ok()?) as usize;
    // e_shstrndx (offset 62, 2 bytes): index of the string table section.
    let e_shstrndx = u16::from_le_bytes(bytes[62..64].try_into().ok()?) as usize;

    if e_shoff == 0 || e_shentsize < ELF64_SHDR_SIZE || e_shnum == 0 {
        tracing::debug!("ELF section header table is invalid or missing");
        return None;
    }
    if e_shstrndx >= e_shnum {
        tracing::warn!(
            "ELF string table index out of bounds: shstrndx={}",
            e_shstrndx
        );
        return None;
    }

    // Validate the overall bounds of the section header table.
    let shdr_table_end = e_shoff.checked_add(e_shentsize.checked_mul(e_shnum)?)?;
    if shdr_table_end > bytes.len() {
        tracing::warn!("ELF section header table exceeds file bounds");
        return None;
    }

    // Read the string table section header at e_shoff + e_shstrndx * e_shentsize.
    let strtab_shdr_off = e_shoff + e_shstrndx * e_shentsize;
    // sh_offset is at section header offset 24 with length 8.
    let strtab_off = u64::from_le_bytes(
        bytes[strtab_shdr_off + 24..strtab_shdr_off + 32]
            .try_into()
            .ok()?,
    ) as usize;
    // sh_size is at section header offset 32 with length 8.
    let strtab_size = u64::from_le_bytes(
        bytes[strtab_shdr_off + 32..strtab_shdr_off + 40]
            .try_into()
            .ok()?,
    ) as usize;
    let strtab_end = strtab_off.checked_add(strtab_size)?;
    if strtab_end > bytes.len() {
        tracing::warn!("ELF string table exceeds file bounds");
        return None;
    }
    let strtab = &bytes[strtab_off..strtab_end];

    // Walk the section header table and find the section named ELF_MANIFEST_SECTION.
    for i in 0..e_shnum {
        let shdr_off = e_shoff + i * e_shentsize;
        if shdr_off + ELF64_SHDR_SIZE > bytes.len() {
            break;
        }

        // sh_name: offset into the string table for this section name (offset 0, 4 bytes).
        let sh_name = u32::from_le_bytes(bytes[shdr_off..shdr_off + 4].try_into().ok()?) as usize;
        // sh_offset (offset 24, 8 bytes): file offset of the section data.
        let sh_off =
            u64::from_le_bytes(bytes[shdr_off + 24..shdr_off + 32].try_into().ok()?) as usize;
        // sh_size (offset 32, 8 bytes): size of the section data.
        let sh_size =
            u64::from_le_bytes(bytes[shdr_off + 32..shdr_off + 40].try_into().ok()?) as usize;

        if sh_name >= strtab.len() {
            continue;
        }
        // Read the null-terminated section name from the string table.
        let name_bytes = &strtab[sh_name..];
        let name_end = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        let Ok(name) = std::str::from_utf8(&name_bytes[..name_end]) else {
            continue;
        };

        if name == ELF_MANIFEST_SECTION {
            let data_end = sh_off.checked_add(sh_size)?;
            if data_end > bytes.len() {
                tracing::warn!("ELF actr_manifest section data exceeds file bounds");
                return None;
            }
            return Some((sh_off, sh_size));
        }
    }

    None
}

pub fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == b"\x7fELF"
}

// ─── Mach-O section parsing ─────────────────────────────────────────────────

/// Mach-O `__TEXT` segment name.
const MACHO_SEGMENT_NAME: &str = "__TEXT";
/// Mach-O manifest section name, limited to 16 bytes.
const MACHO_SECTION_NAME: &str = "__actr_mani";

/// Mach-O 64-bit little-endian magic, stored as `CF FA ED FE`.
const MACHO_MAGIC_64_LE: u32 = 0xFEED_FACF;
/// Mach-O fat binary magic in big-endian form (`BE FE ED FA`).
const MACHO_FAT_MAGIC: u32 = 0xCAFE_BABE;

/// Mach-O load command type: `LC_SEGMENT_64`.
const LC_SEGMENT_64: u32 = 0x19;

/// Extract the manifest section payload from Mach-O bytes.
///
/// Supports only 64-bit little-endian binaries (x86_64, arm64).
/// Fat binaries are not supported and return `None` with a warning.
/// Returns the payload slice of `__TEXT/__actr_mani`, or `None` if not found.
pub fn extract_macho_manifest(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 4 {
        return None;
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().ok()?);

    // Detect fat binaries using the big-endian magic.
    let magic_be = u32::from_be_bytes(bytes[0..4].try_into().ok()?);
    if magic_be == MACHO_FAT_MAGIC {
        tracing::warn!(
            "Detected a Mach-O fat binary; run `lipo -thin <arch>` first to extract a single-architecture slice"
        );
        return None;
    }

    if magic != MACHO_MAGIC_64_LE {
        return None;
    }

    let (data_off, data_size) = macho_manifest_data_range_inner(bytes)?;
    let data_end = data_off.checked_add(data_size)?;
    if data_end > bytes.len() {
        return None;
    }
    tracing::debug!(
        section_offset = data_off,
        section_size = data_size,
        "Found Mach-O __TEXT/__actr_mani section"
    );
    Some(&bytes[data_off..data_end])
}

/// Compute the SHA-256 hash of a Mach-O file after zero-filling the manifest section.
///
/// Fat binaries are explicitly rejected with a descriptive error.
pub fn macho_binary_hash_excluding_manifest(bytes: &[u8]) -> HyperResult<[u8; 32]> {
    if bytes.len() < 4 {
        return Err(HyperError::InvalidManifest(
            "Not a valid Mach-O file (too short)".to_string(),
        ));
    }
    let magic_be = u32::from_be_bytes(
        bytes[0..4]
            .try_into()
            .map_err(|_| HyperError::InvalidManifest("Failed to read Mach-O magic".to_string()))?,
    );
    if magic_be == MACHO_FAT_MAGIC {
        return Err(HyperError::InvalidManifest(
            "Mach-O fat binaries are not supported yet; run `lipo -thin <arch>` first".to_string(),
        ));
    }

    let magic = u32::from_le_bytes(
        bytes[0..4]
            .try_into()
            .map_err(|_| HyperError::InvalidManifest("Failed to read Mach-O magic".to_string()))?,
    );
    if magic != MACHO_MAGIC_64_LE {
        return Err(HyperError::InvalidManifest(
            "Not a valid 64-bit little-endian Mach-O file".to_string(),
        ));
    }

    let manifest_range = macho_manifest_data_range(bytes)?;

    let mut hasher = Sha256::new();
    if let Some((data_off, data_size)) = manifest_range {
        // Zero-fill the manifest section data and hash the rest normally.
        hasher.update(&bytes[..data_off]);
        hasher.update(vec![0u8; data_size]);
        hasher.update(&bytes[data_off + data_size..]);
    } else {
        tracing::debug!("No __actr_mani section found in Mach-O; hashing the entire file");
        hasher.update(bytes);
    }

    Ok(hasher.finalize().into())
}

/// Return `(file_offset, size)` for the Mach-O `__TEXT/__actr_mani` section.
pub fn macho_manifest_data_range(bytes: &[u8]) -> HyperResult<Option<(usize, usize)>> {
    Ok(macho_manifest_data_range_inner(bytes))
}

/// Internal `Option`-based lookup for the Mach-O `__TEXT/__actr_mani` section.
fn macho_manifest_data_range_inner(bytes: &[u8]) -> Option<(usize, usize)> {
    // mach_header_64 layout (32 bytes):
    //   magic(4) + cputype(4) + cpusubtype(4) + filetype(4) + ncmds(4) + sizeofcmds(4) + flags(4) + reserved(4)
    if bytes.len() < 32 {
        return None;
    }

    let ncmds = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
    let sizeofcmds = u32::from_le_bytes(bytes[20..24].try_into().ok()?) as usize;

    // Load commands start immediately after mach_header_64 (32 bytes).
    let lc_start = 32usize;
    let lc_end = lc_start.checked_add(sizeofcmds)?;
    if lc_end > bytes.len() {
        tracing::warn!("Mach-O load commands exceed file bounds");
        return None;
    }

    let mut pos = lc_start;
    for _ in 0..ncmds {
        if pos + 8 > lc_end {
            break;
        }
        let cmd = u32::from_le_bytes(bytes[pos..pos + 4].try_into().ok()?);
        let cmdsize = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;

        if cmdsize == 0 {
            // Prevent infinite loops.
            break;
        }

        if cmd == LC_SEGMENT_64 {
            // segment_command_64 layout (72 bytes):
            //   cmd(4) + cmdsize(4) + segname[16] + vmaddr(8) + vmsize(8) + fileoff(8) + filesize(8)
            //   + maxprot(4) + initprot(4) + nsects(4) + flags(4)
            // segname starts at offset 8, uses 16 bytes, and is null-padded.
            if pos + 72 > lc_end {
                pos += cmdsize;
                continue;
            }

            let segname_bytes = &bytes[pos + 8..pos + 24];
            let segname_end = segname_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(segname_bytes.len());
            let Ok(segname) = std::str::from_utf8(&segname_bytes[..segname_end]) else {
                pos += cmdsize;
                continue;
            };

            if segname == MACHO_SEGMENT_NAME {
                // nsects is at segment_command_64 offset 64 (4 bytes).
                let nsects =
                    u32::from_le_bytes(bytes[pos + 64..pos + 68].try_into().ok()?) as usize;

                // section_64 entries follow segment_command_64 (72 bytes), 80 bytes each:
                //   sectname[16] + segname[16] + addr(8) + size(8) + offset(4) + align(4)
                //   + reloff(4) + nreloc(4) + flags(4) + reserved1(4) + reserved2(4) + reserved3(4)
                let sections_start = pos + 72;
                for s in 0..nsects {
                    let sect_base = sections_start + s * 80;
                    if sect_base + 80 > lc_end {
                        break;
                    }

                    let sectname_bytes = &bytes[sect_base..sect_base + 16];
                    let sectname_end = sectname_bytes
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(sectname_bytes.len());
                    let Ok(sectname) = std::str::from_utf8(&sectname_bytes[..sectname_end]) else {
                        continue;
                    };

                    if sectname == MACHO_SECTION_NAME {
                        // size (offset 40, 8 bytes): section data size.
                        let sect_size = u64::from_le_bytes(
                            bytes[sect_base + 40..sect_base + 48].try_into().ok()?,
                        ) as usize;
                        // offset (offset 48, 4 bytes): file offset of the section data.
                        let sect_fileoff = u32::from_le_bytes(
                            bytes[sect_base + 48..sect_base + 52].try_into().ok()?,
                        ) as usize;

                        let data_end = sect_fileoff.checked_add(sect_size)?;
                        if data_end > bytes.len() {
                            tracing::warn!("Mach-O __actr_mani section exceeds file bounds");
                            return None;
                        }
                        return Some((sect_fileoff, sect_size));
                    }
                }
            }
        }

        pos = pos.checked_add(cmdsize)?;
    }

    None
}

pub fn is_macho(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    // Fat binaries are not treated as Mach-O here; let the caller distinguish them.
    match bytes[0..4].try_into().map(u32::from_le_bytes) {
        Ok(magic) => magic == MACHO_MAGIC_64_LE,
        Err(_) => false,
    }
}

/// Parse an unsigned 32-bit integer encoded as LEB128.
///
/// Returns `(value, bytes_consumed)` or `None` on failure.
pub(crate) fn read_leb128_u32(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut result = 0u32;
    let mut shift = 0u32;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 32 {
            return None; // Overflow.
        }
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None // Unterminated LEB128.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leb128_single_byte() {
        assert_eq!(read_leb128_u32(&[0x05]), Some((5, 1)));
        assert_eq!(read_leb128_u32(&[0x00]), Some((0, 1)));
        assert_eq!(read_leb128_u32(&[0x7F]), Some((127, 1)));
    }

    #[test]
    fn leb128_multi_byte() {
        // 128 = 0x80 0x01
        assert_eq!(read_leb128_u32(&[0x80, 0x01]), Some((128, 2)));
    }

    #[test]
    fn leb128_empty_returns_none() {
        assert_eq!(read_leb128_u32(&[]), None);
    }

    #[test]
    fn not_wasm_returns_none() {
        let not_wasm = b"hello world";
        assert!(extract_wasm_manifest(not_wasm).is_none());
    }

    #[test]
    fn binary_hash_rejects_non_wasm() {
        let result = wasm_binary_hash_excluding_manifest(b"not wasm");
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    // ─── ELF test helpers ────────────────────────────────────────────────────

    /// Build minimal valid ELF64 little-endian bytes with an `actr_manifest` section.
    ///
    /// Layout:
    ///   [0..64]   ELF header
    ///   [64..64+payload_len]  section data (manifest payload)
    ///   [64+payload_len..]    section header table (2 entries: null + actr_manifest)
    ///                          + string table data
    ///
    /// String table contents: `"\0actr_manifest\0"`.
    fn build_minimal_elf(payload: &[u8]) -> Vec<u8> {
        // String table: \0 + "actr_manifest" + \0.
        // "\0" occupies index 0 (SHN_UNDEF name); "actr_manifest" starts at index 1.
        let strtab: Vec<u8> = {
            let mut v = vec![0u8]; // Index 0: empty string (null section name).
            v.extend_from_slice(b"actr_manifest\0"); // Index 1: manifest section name.
            v
        };
        let strtab_name_idx: u32 = 1; // Index of "actr_manifest" in the string table.

        let ehdr_size: u64 = 64;
        let shdr_size: u64 = 64;
        let payload_len = payload.len() as u64;

        // Section data immediately follows the ELF header.
        let manifest_data_off: u64 = ehdr_size;
        // The section header table follows the manifest data.
        let shoff: u64 = ehdr_size + payload_len;
        // Three section headers: null + actr_manifest + strtab.
        let shnum: u16 = 3;
        let shstrndx: u16 = 2; // String table is section 2 (0-based).
        // String table data follows the section header table.
        let strtab_off: u64 = shoff + shdr_size * shnum as u64;

        let mut buf = Vec::new();

        // ── ELF header (64 bytes) ──
        buf.extend_from_slice(b"\x7fELF"); // magic
        buf.push(2); // EI_CLASS: ELF64
        buf.push(1); // EI_DATA: LE
        buf.push(1); // EI_VERSION
        buf.push(0); // EI_OSABI
        buf.extend_from_slice(&[0u8; 8]); // EI_ABIVERSION + padding
        buf.extend_from_slice(&2u16.to_le_bytes()); // e_type: ET_EXEC
        buf.extend_from_slice(&62u16.to_le_bytes()); // e_machine: x86_64
        buf.extend_from_slice(&1u32.to_le_bytes()); // e_version
        buf.extend_from_slice(&0u64.to_le_bytes()); // e_entry
        buf.extend_from_slice(&0u64.to_le_bytes()); // e_phoff
        buf.extend_from_slice(&shoff.to_le_bytes()); // e_shoff
        buf.extend_from_slice(&0u32.to_le_bytes()); // e_flags
        buf.extend_from_slice(&(ehdr_size as u16).to_le_bytes()); // e_ehsize
        buf.extend_from_slice(&0u16.to_le_bytes()); // e_phentsize
        buf.extend_from_slice(&0u16.to_le_bytes()); // e_phnum
        buf.extend_from_slice(&(shdr_size as u16).to_le_bytes()); // e_shentsize
        buf.extend_from_slice(&shnum.to_le_bytes()); // e_shnum
        buf.extend_from_slice(&shstrndx.to_le_bytes()); // e_shstrndx
        assert_eq!(buf.len(), 64);

        // ── Manifest section data ──
        buf.extend_from_slice(payload);

        // ── section header table ──
        // [0] SHN_UNDEF (null section)
        buf.extend_from_slice(&[0u8; 64]);

        // [1] actr_manifest section header
        buf.extend_from_slice(&strtab_name_idx.to_le_bytes()); // sh_name
        buf.extend_from_slice(&1u32.to_le_bytes()); // sh_type: SHT_PROGBITS
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_flags
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_addr
        buf.extend_from_slice(&manifest_data_off.to_le_bytes()); // sh_offset
        buf.extend_from_slice(&payload_len.to_le_bytes()); // sh_size
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_link
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_info
        buf.extend_from_slice(&1u64.to_le_bytes()); // sh_addralign
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_entsize

        // [2] String table section header
        let strtab_len = strtab.len() as u64;
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_name (points to index 0: empty string)
        buf.extend_from_slice(&3u32.to_le_bytes()); // sh_type: SHT_STRTAB
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_flags
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_addr
        buf.extend_from_slice(&strtab_off.to_le_bytes()); // sh_offset
        buf.extend_from_slice(&strtab_len.to_le_bytes()); // sh_size
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_link
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_info
        buf.extend_from_slice(&1u64.to_le_bytes()); // sh_addralign
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_entsize

        // ── String table data ──
        buf.extend_from_slice(&strtab);

        buf
    }

    #[test]
    fn elf_extract_manifest_found() {
        let payload = b"hello elf manifest";
        let elf = build_minimal_elf(payload);
        let extracted = extract_elf_manifest(&elf);
        assert_eq!(extracted, Some(payload.as_ref()));
    }

    #[test]
    fn elf_extract_manifest_not_found_on_non_elf() {
        assert!(extract_elf_manifest(b"not an elf").is_none());
        assert!(extract_elf_manifest(b"\x7fELF").is_none()); // Too short.
    }

    #[test]
    fn elf_binary_hash_zero_fills_manifest() {
        let payload = b"manifest data";
        let elf = build_minimal_elf(payload);

        // Manually build the zero-filled variant and verify the hash matches.
        let mut zeroed = elf.clone();
        // Manifest data starts at offset 64 and spans payload.len() bytes.
        let start = 64;
        let end = start + payload.len();
        for b in &mut zeroed[start..end] {
            *b = 0;
        }
        let expected: [u8; 32] = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(&zeroed);
            h.finalize().into()
        };

        let computed = elf_binary_hash_excluding_manifest(&elf).unwrap();
        assert_eq!(computed, expected);
    }

    #[test]
    fn elf_binary_hash_rejects_non_elf() {
        let result = elf_binary_hash_excluding_manifest(b"not elf");
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    #[test]
    fn is_elf_detects_magic() {
        assert!(is_elf(b"\x7fELF...."));
        assert!(!is_elf(b"\0asm"));
        assert!(!is_elf(b""));
    }

    // ─── Mach-O test helpers ─────────────────────────────────────────────────

    /// Build minimal valid Mach-O 64-bit little-endian bytes with `__TEXT/__actr_mani`.
    ///
    /// Layout:
    ///   [0..32]            mach_header_64
    ///   [32..32+lc_size]   LC_SEGMENT_64 load command (including section_64)
    ///   [32+lc_size..]     section data (manifest payload)
    fn build_minimal_macho(payload: &[u8]) -> Vec<u8> {
        // mach_header_64 = 32 bytes
        // segment_command_64 = 72 bytes
        // section_64 = 80 bytes
        // Total load command size = 72 + 80 = 152 bytes
        let lc_size: u32 = 72 + 80;
        let header_size: usize = 32;
        let data_off: u32 = (header_size as u32) + lc_size;
        let payload_len = payload.len();

        let mut buf = Vec::new();

        // ── mach_header_64 (32 bytes) ──
        buf.extend_from_slice(&MACHO_MAGIC_64_LE.to_le_bytes()); // magic
        buf.extend_from_slice(&0x0100_000cu32.to_le_bytes()); // cputype: ARM64
        buf.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
        buf.extend_from_slice(&2u32.to_le_bytes()); // filetype: MH_EXECUTE
        buf.extend_from_slice(&1u32.to_le_bytes()); // ncmds: one load command
        buf.extend_from_slice(&lc_size.to_le_bytes()); // sizeofcmds
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(buf.len(), 32);

        // ── segment_command_64 (72 bytes) ──
        buf.extend_from_slice(&LC_SEGMENT_64.to_le_bytes()); // cmd
        buf.extend_from_slice(&lc_size.to_le_bytes()); // cmdsize

        // segname[16]："__TEXT\0\0\0\0\0\0\0\0\0\0"
        let mut segname = [0u8; 16];
        segname[..6].copy_from_slice(b"__TEXT");
        buf.extend_from_slice(&segname);

        buf.extend_from_slice(&0u64.to_le_bytes()); // vmaddr
        buf.extend_from_slice(&(payload_len as u64).to_le_bytes()); // vmsize
        buf.extend_from_slice(&(data_off as u64).to_le_bytes()); // fileoff
        buf.extend_from_slice(&(payload_len as u64).to_le_bytes()); // filesize
        buf.extend_from_slice(&7i32.to_le_bytes()); // maxprot
        buf.extend_from_slice(&5i32.to_le_bytes()); // initprot
        buf.extend_from_slice(&1u32.to_le_bytes()); // nsects: one section
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        assert_eq!(buf.len(), 32 + 72);

        // ── section_64 (80 bytes) ──
        // sectname[16]: "__actr_mani\0\0\0\0\0"
        let mut sectname = [0u8; 16];
        sectname[..11].copy_from_slice(b"__actr_mani");
        buf.extend_from_slice(&sectname);

        // segname[16]
        buf.extend_from_slice(&segname);

        buf.extend_from_slice(&0u64.to_le_bytes()); // addr
        buf.extend_from_slice(&(payload_len as u64).to_le_bytes()); // size (offset 40)
        buf.extend_from_slice(&data_off.to_le_bytes()); // offset (offset 48)
        buf.extend_from_slice(&0u32.to_le_bytes()); // align
        buf.extend_from_slice(&0u32.to_le_bytes()); // reloff
        buf.extend_from_slice(&0u32.to_le_bytes()); // nreloc
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved2
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved3
        assert_eq!(buf.len(), 32 + 72 + 80);

        // ── Section data ──
        buf.extend_from_slice(payload);

        buf
    }

    #[test]
    fn macho_extract_manifest_found() {
        let payload = b"hello macho manifest";
        let macho = build_minimal_macho(payload);
        let extracted = extract_macho_manifest(&macho);
        assert_eq!(extracted, Some(payload.as_ref()));
    }

    #[test]
    fn macho_extract_manifest_not_found_on_non_macho() {
        assert!(extract_macho_manifest(b"not macho").is_none());
        assert!(extract_macho_manifest(b"").is_none());
    }

    #[test]
    fn macho_fat_binary_returns_none() {
        // Fat binary magic in big-endian form: CA FE BA BE.
        let fat_bytes = &[0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 1];
        assert!(extract_macho_manifest(fat_bytes).is_none());
    }

    #[test]
    fn macho_binary_hash_rejects_fat() {
        let fat_bytes = &[0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 1];
        let result = macho_binary_hash_excluding_manifest(fat_bytes);
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    #[test]
    fn macho_binary_hash_rejects_non_macho() {
        let result = macho_binary_hash_excluding_manifest(b"not macho");
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    #[test]
    fn macho_binary_hash_zero_fills_manifest() {
        let payload = b"macho manifest data";
        let macho = build_minimal_macho(payload);

        // Manifest data starts at header_size + lc_size = 32 + 152 = 184.
        let data_off = 32 + 72 + 80; // = 184
        let mut zeroed = macho.clone();
        for b in &mut zeroed[data_off..data_off + payload.len()] {
            *b = 0;
        }
        let expected: [u8; 32] = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(&zeroed);
            h.finalize().into()
        };

        let computed = macho_binary_hash_excluding_manifest(&macho).unwrap();
        assert_eq!(computed, expected);
    }

    #[test]
    fn is_macho_detects_magic() {
        // MACHO_MAGIC_64_LE = 0xFEEDFACF, stored in little-endian as CF FA ED FE.
        let magic_le: [u8; 4] = MACHO_MAGIC_64_LE.to_le_bytes();
        let mut bytes = vec![0u8; 8];
        bytes[..4].copy_from_slice(&magic_le);
        assert!(is_macho(&bytes));
        assert!(!is_macho(b"\0asm"));
        assert!(!is_macho(b""));
    }
}
