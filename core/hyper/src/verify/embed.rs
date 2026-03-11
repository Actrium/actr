//! Embed manifest JSON into actor package binaries.
//!
//! Used by `actr dev sign` to write the signed manifest JSON into the target
//! binary section.
//!
//! | Format | Section name | Implementation |
//! |--------|--------------|----------------|
//! | WASM | custom `actr-manifest` | pure Rust |
//! | ELF64 | `.actr_manifest` | `objcopy` subprocess |
//! | Mach-O | `__TEXT/__actr_mani` | `llvm-objcopy` subprocess |

use std::path::Path;

use super::manifest::{WASM_MANIFEST_SECTION, is_wasm, read_leb128_u32};
use crate::error::{HyperError, HyperResult};

// Re-export section name constants so embed/extract stay aligned.
pub const ELF_SECTION_NAME: &str = "actr_manifest";
pub const MACHO_SEGMENT_NAME: &str = "__TEXT";
pub const MACHO_SECTION_NAME: &str = "__actr_mani";

// ─── WASM ────────────────────────────────────────────────────────────────────

/// Embed manifest JSON into a WASM byte stream.
///
/// If an `actr-manifest` custom section already exists, it is removed before
/// appending the new payload. The new section is always appended after the
/// final section.
pub fn embed_wasm_manifest(wasm_bytes: &[u8], manifest_json: &[u8]) -> HyperResult<Vec<u8>> {
    if !is_wasm(wasm_bytes) {
        return Err(HyperError::InvalidManifest(
            "Not a valid WASM file".to_string(),
        ));
    }

    // Rebuild the byte stream while skipping the old actr-manifest section.
    let mut out = Vec::with_capacity(wasm_bytes.len() + manifest_json.len() + 32);
    out.extend_from_slice(&wasm_bytes[0..8]); // magic + version

    let mut pos = 8usize;
    while pos < wasm_bytes.len() {
        let section_id_pos = pos;
        let section_id = wasm_bytes[pos];
        pos += 1;

        let (section_size, bytes_read) = read_leb128_u32(&wasm_bytes[pos..]).ok_or_else(|| {
            HyperError::InvalidManifest("Failed to decode WASM LEB128".to_string())
        })?;
        let header_end = pos + bytes_read;
        let section_end = header_end + section_size as usize;

        if section_end > wasm_bytes.len() {
            return Err(HyperError::InvalidManifest(
                "WASM section exceeds file bounds".to_string(),
            ));
        }

        // Skip the old actr-manifest custom section.
        let mut skip = false;
        if section_id == 0 {
            if let Some((name_len, name_leb_bytes)) = read_leb128_u32(&wasm_bytes[header_end..]) {
                let name_start = header_end + name_leb_bytes;
                let name_end = name_start + name_len as usize;
                if name_end <= section_end {
                    if let Ok(name) = std::str::from_utf8(&wasm_bytes[name_start..name_end]) {
                        if name == WASM_MANIFEST_SECTION {
                            skip = true;
                        }
                    }
                }
            }
        }

        if !skip {
            out.extend_from_slice(&wasm_bytes[section_id_pos..section_end]);
        }
        pos = section_end;
    }

    // Append the new actr-manifest custom section.
    let name_bytes = WASM_MANIFEST_SECTION.as_bytes();
    let name_len_enc = encode_leb128_u32(name_bytes.len() as u32);
    let payload_len = name_len_enc.len() + name_bytes.len() + manifest_json.len();
    let section_size_enc = encode_leb128_u32(payload_len as u32);

    out.push(0x00); // custom section id
    out.extend_from_slice(&section_size_enc);
    out.extend_from_slice(&name_len_enc);
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(manifest_json);

    tracing::info!(
        manifest_len = manifest_json.len(),
        output_len = out.len(),
        "Embedded WASM manifest section"
    );
    Ok(out)
}

// ─── ELF (via objcopy subprocess) ───────────────────────────────────────────

/// Embed manifest JSON into an ELF binary.
///
/// Uses the `objcopy` subprocess and requires `binutils` to be installed.
/// Removes any existing `.actr_manifest` section first so the operation is
/// idempotent.
pub fn embed_elf_manifest(
    input_path: &Path,
    output_path: &Path,
    manifest_json: &[u8],
) -> HyperResult<()> {
    // Write the manifest JSON to a temporary file.
    let tmp = tempfile_for_manifest(manifest_json)?;

    // Check whether objcopy is available.
    let objcopy = find_objcopy()?;

    tracing::debug!(
        binary = %input_path.display(),
        tool = %objcopy,
        "Embedding ELF .actr_manifest section with objcopy"
    );

    // objcopy --remove-section=.actr_manifest --add-section=.actr_manifest=<tmp> \
    //         --set-section-flags=.actr_manifest=readonly <input> <output>
    let status = std::process::Command::new(&objcopy)
        .arg(format!("--remove-section={ELF_SECTION_NAME}"))
        .arg(format!(
            "--add-section={ELF_SECTION_NAME}={}",
            tmp.path().display()
        ))
        .arg(format!("--set-section-flags={ELF_SECTION_NAME}=readonly"))
        .arg(input_path)
        .arg(output_path)
        .status()
        .map_err(|e| {
            HyperError::Runtime(format!(
                "Failed to start objcopy (is binutils installed?): {e}"
            ))
        })?;

    if !status.success() {
        return Err(HyperError::Runtime(format!(
            "objcopy failed with exit code {:?}",
            status.code()
        )));
    }

    tracing::info!(
        binary = %output_path.display(),
        "Embedded ELF .actr_manifest section"
    );
    Ok(())
}

// ─── Mach-O (via llvm-objcopy subprocess) ───────────────────────────────────

/// Embed manifest JSON into a Mach-O binary.
///
/// Uses the `llvm-objcopy` subprocess and requires the LLVM toolchain.
/// The manifest is embedded in the `__TEXT/__actr_mani` section.
pub fn embed_macho_manifest(
    input_path: &Path,
    output_path: &Path,
    manifest_json: &[u8],
) -> HyperResult<()> {
    let tmp = tempfile_for_manifest(manifest_json)?;

    // llvm-objcopy may be installed under different names.
    let tool = find_llvm_objcopy()?;

    tracing::debug!(
        binary = %input_path.display(),
        tool = %tool,
        "Embedding Mach-O __TEXT/__actr_mani section with llvm-objcopy"
    );

    // llvm-objcopy --remove-section=__TEXT,__actr_mani \
    //              --add-section=__TEXT,__actr_mani=<tmp> <input> <output>
    let section_spec = format!("{MACHO_SEGMENT_NAME},{MACHO_SECTION_NAME}");
    let status = std::process::Command::new(&tool)
        .arg(format!("--remove-section={section_spec}"))
        .arg(format!(
            "--add-section={section_spec}={}",
            tmp.path().display()
        ))
        .arg(input_path)
        .arg(output_path)
        .status()
        .map_err(|e| {
            HyperError::Runtime(format!(
                "Failed to start llvm-objcopy (is the LLVM toolchain installed?): {e}"
            ))
        })?;

    if !status.success() {
        return Err(HyperError::Runtime(format!(
            "llvm-objcopy failed with exit code {:?}",
            status.code()
        )));
    }

    tracing::info!(
        binary = %output_path.display(),
        "Embedded Mach-O __TEXT/__actr_mani section"
    );
    Ok(())
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Encode an unsigned 32-bit integer as LEB128.
pub(crate) fn encode_leb128_u32(mut value: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            out.push(byte | 0x80);
        } else {
            out.push(byte);
            break;
        }
    }
    out
}

/// Write manifest JSON to a temporary file and keep it open to avoid early deletion.
fn tempfile_for_manifest(manifest_json: &[u8]) -> HyperResult<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new()
        .map_err(|e| HyperError::Runtime(format!("Failed to create temp file: {e}")))?;
    tmp.write_all(manifest_json)
        .map_err(|e| HyperError::Runtime(format!("Failed to write temp file: {e}")))?;
    tmp.flush()
        .map_err(|e| HyperError::Runtime(format!("Failed to flush temp file: {e}")))?;
    Ok(tmp)
}

/// Find the objcopy executable from binutils.
fn find_objcopy() -> HyperResult<String> {
    for candidate in &["objcopy", "llvm-objcopy", "x86_64-linux-gnu-objcopy"] {
        if std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok()
        {
            return Ok(candidate.to_string());
        }
    }
    Err(HyperError::Runtime(
        "objcopy not found; install binutils (Ubuntu: apt install binutils)".to_string(),
    ))
}

/// Find the llvm-objcopy executable.
fn find_llvm_objcopy() -> HyperResult<String> {
    for candidate in &[
        "llvm-objcopy",
        "llvm-objcopy-18",
        "llvm-objcopy-17",
        "llvm-objcopy-16",
    ] {
        if std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok()
        {
            return Ok(candidate.to_string());
        }
    }
    Err(HyperError::Runtime(
        "llvm-objcopy not found; install the LLVM toolchain (macOS: brew install llvm)".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::manifest::{extract_wasm_manifest, wasm_binary_hash_excluding_manifest};

    fn minimal_wasm() -> Vec<u8> {
        // Minimal valid WASM: magic + version, no sections.
        b"\0asm\x01\x00\x00\x00".to_vec()
    }

    #[test]
    fn encode_leb128_single_byte() {
        assert_eq!(encode_leb128_u32(0), vec![0x00]);
        assert_eq!(encode_leb128_u32(1), vec![0x01]);
        assert_eq!(encode_leb128_u32(127), vec![0x7F]);
    }

    #[test]
    fn encode_leb128_multi_byte() {
        assert_eq!(encode_leb128_u32(128), vec![0x80, 0x01]);
        assert_eq!(encode_leb128_u32(300), vec![0xAC, 0x02]);
    }

    #[test]
    fn embed_wasm_roundtrip() {
        let wasm = minimal_wasm();
        let manifest_json = br#"{"manufacturer":"test","actr_name":"A","version":"1","binary_hash":"00","capabilities":[],"signature":"AA=="}"#;

        let embedded = embed_wasm_manifest(&wasm, manifest_json).unwrap();

        // The embedded output must still be valid WASM (same magic).
        assert_eq!(&embedded[0..4], b"\0asm");

        // The manifest section should be extractable.
        let extracted = extract_wasm_manifest(&embedded).unwrap();
        assert_eq!(extracted, manifest_json);
    }

    #[test]
    fn embed_wasm_replaces_existing_section() {
        let wasm = minimal_wasm();
        let first = br#"{"v":"1"}"#;
        let second = br#"{"v":"2","extra":"data"}"#;

        let after_first = embed_wasm_manifest(&wasm, first).unwrap();
        let after_second = embed_wasm_manifest(&after_first, second).unwrap();

        let extracted = extract_wasm_manifest(&after_second).unwrap();
        assert_eq!(
            extracted, second,
            "The second embed should replace the first section"
        );
    }

    #[test]
    fn embed_wasm_hash_excludes_section() {
        let wasm = minimal_wasm();
        let manifest_json = b"test-manifest-content";

        let hash_before = wasm_binary_hash_excluding_manifest(&wasm).unwrap();
        let embedded = embed_wasm_manifest(&wasm, manifest_json).unwrap();
        let hash_after = wasm_binary_hash_excluding_manifest(&embedded).unwrap();

        assert_eq!(
            hash_before, hash_after,
            "binary_hash should remain unchanged after embedding the manifest section"
        );
    }

    #[test]
    fn embed_wasm_rejects_non_wasm() {
        let result = embed_wasm_manifest(b"ELF content", b"{}");
        assert!(result.is_err());
    }
}
