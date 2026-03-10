//! 将 manifest JSON 嵌入 Actor 包二进制文件
//!
//! 供 `actr dev sign` 工具使用，将签名后的 manifest JSON 写入对应 section。
//!
//! | 格式    | section 名            | 实现方式         |
//! |---------|-----------------------|-----------------|
//! | WASM    | custom `actr-manifest` | 纯 Rust         |
//! | ELF64   | `.actr_manifest`      | `objcopy` 子进程 |
//! | Mach-O  | `__TEXT/__actr_mani`  | `llvm-objcopy` 子进程 |

use std::path::Path;

use crate::error::{HyperError, HyperResult};
use super::manifest::{WASM_MANIFEST_SECTION, read_leb128_u32, is_wasm};

// 重新导出 section 名称常量（embed 逻辑需要与 extract 保持一致）
pub const ELF_SECTION_NAME: &str = "actr_manifest";
pub const MACHO_SEGMENT_NAME: &str = "__TEXT";
pub const MACHO_SECTION_NAME: &str = "__actr_mani";

// ─── WASM ────────────────────────────────────────────────────────────────────

/// 将 manifest JSON 嵌入 WASM 字节流
///
/// 若已存在 `actr-manifest` custom section，先移除后再追加新内容。
/// 新 section 始终追加在最后一个 section 之后。
pub fn embed_wasm_manifest(wasm_bytes: &[u8], manifest_json: &[u8]) -> HyperResult<Vec<u8>> {
    if !is_wasm(wasm_bytes) {
        return Err(HyperError::InvalidManifest(
            "不是有效的 WASM 文件".to_string(),
        ));
    }

    // 重建字节流，跳过旧的 actr-manifest section
    let mut out = Vec::with_capacity(wasm_bytes.len() + manifest_json.len() + 32);
    out.extend_from_slice(&wasm_bytes[0..8]); // magic + version

    let mut pos = 8usize;
    while pos < wasm_bytes.len() {
        let section_id_pos = pos;
        let section_id = wasm_bytes[pos];
        pos += 1;

        let (section_size, bytes_read) = read_leb128_u32(&wasm_bytes[pos..])
            .ok_or_else(|| HyperError::InvalidManifest("WASM LEB128 解码失败".to_string()))?;
        let header_end = pos + bytes_read;
        let section_end = header_end + section_size as usize;

        if section_end > wasm_bytes.len() {
            return Err(HyperError::InvalidManifest(
                "WASM section 超出文件边界".to_string(),
            ));
        }

        // 跳过旧的 actr-manifest custom section
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

    // 追加新的 actr-manifest custom section
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
        "WASM manifest section 已嵌入"
    );
    Ok(out)
}

// ─── ELF（通过 objcopy 子进程）────────────────────────────────────────────────

/// 将 manifest JSON 嵌入 ELF 二进制文件
///
/// 使用 `objcopy` 子进程实现，需要系统安装 `binutils`（Linux 标准工具）。
/// 先移除已有的 `.actr_manifest` section，再重新添加，保证幂等。
pub fn embed_elf_manifest(
    input_path: &Path,
    output_path: &Path,
    manifest_json: &[u8],
) -> HyperResult<()> {
    // 将 manifest JSON 写入临时文件
    let tmp = tempfile_for_manifest(manifest_json)?;

    // 检测 objcopy 是否可用
    let objcopy = find_objcopy()?;

    tracing::debug!(
        binary = %input_path.display(),
        tool = %objcopy,
        "使用 objcopy 嵌入 ELF .actr_manifest section"
    );

    // objcopy --remove-section=.actr_manifest --add-section=.actr_manifest=<tmp> \
    //         --set-section-flags=.actr_manifest=readonly <input> <output>
    let status = std::process::Command::new(&objcopy)
        .arg(format!("--remove-section={ELF_SECTION_NAME}"))
        .arg(format!(
            "--add-section={ELF_SECTION_NAME}={}",
            tmp.path().display()
        ))
        .arg(format!(
            "--set-section-flags={ELF_SECTION_NAME}=readonly"
        ))
        .arg(input_path)
        .arg(output_path)
        .status()
        .map_err(|e| {
            HyperError::Runtime(format!("objcopy 启动失败（是否已安装 binutils？）: {e}"))
        })?;

    if !status.success() {
        return Err(HyperError::Runtime(format!(
            "objcopy 失败，exit code: {:?}",
            status.code()
        )));
    }

    tracing::info!(
        binary = %output_path.display(),
        "ELF .actr_manifest section 已嵌入"
    );
    Ok(())
}

// ─── Mach-O（通过 llvm-objcopy 子进程）────────────────────────────────────────

/// 将 manifest JSON 嵌入 Mach-O 二进制文件
///
/// 使用 `llvm-objcopy` 子进程实现，需要安装 LLVM 工具链。
/// 嵌入的 section 位于 `__TEXT/__actr_mani`。
pub fn embed_macho_manifest(
    input_path: &Path,
    output_path: &Path,
    manifest_json: &[u8],
) -> HyperResult<()> {
    let tmp = tempfile_for_manifest(manifest_json)?;

    // llvm-objcopy 可能以不同名称存在
    let tool = find_llvm_objcopy()?;

    tracing::debug!(
        binary = %input_path.display(),
        tool = %tool,
        "使用 llvm-objcopy 嵌入 Mach-O __TEXT/__actr_mani section"
    );

    // llvm-objcopy --remove-section=__TEXT,__actr_mani \
    //              --add-section=__TEXT,__actr_mani=<tmp> <input> <output>
    let section_spec = format!("{MACHO_SEGMENT_NAME},{MACHO_SECTION_NAME}");
    let status = std::process::Command::new(&tool)
        .arg(format!("--remove-section={section_spec}"))
        .arg(format!("--add-section={section_spec}={}", tmp.path().display()))
        .arg(input_path)
        .arg(output_path)
        .status()
        .map_err(|e| {
            HyperError::Runtime(format!(
                "llvm-objcopy 启动失败（是否已安装 LLVM 工具链？）: {e}"
            ))
        })?;

    if !status.success() {
        return Err(HyperError::Runtime(format!(
            "llvm-objcopy 失败，exit code: {:?}",
            status.code()
        )));
    }

    tracing::info!(
        binary = %output_path.display(),
        "Mach-O __TEXT/__actr_mani section 已嵌入"
    );
    Ok(())
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// LEB128 编码无符号 32 位整数
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

/// 将 manifest JSON 写入临时文件，返回 NamedTempFile（保持文件打开以防提前删除）
fn tempfile_for_manifest(manifest_json: &[u8]) -> HyperResult<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new()
        .map_err(|e| HyperError::Runtime(format!("创建临时文件失败: {e}")))?;
    tmp.write_all(manifest_json)
        .map_err(|e| HyperError::Runtime(format!("写入临时文件失败: {e}")))?;
    tmp.flush()
        .map_err(|e| HyperError::Runtime(format!("刷新临时文件失败: {e}")))?;
    Ok(tmp)
}

/// 查找 objcopy（binutils）路径
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
        "未找到 objcopy 工具，请安装 binutils（Ubuntu: apt install binutils）".to_string(),
    ))
}

/// 查找 llvm-objcopy 路径
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
        "未找到 llvm-objcopy 工具，请安装 LLVM 工具链（macOS: brew install llvm）".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::manifest::{extract_wasm_manifest, wasm_binary_hash_excluding_manifest};

    fn minimal_wasm() -> Vec<u8> {
        // 最小合法 WASM: magic + version，无 section
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

        // 嵌入后仍是合法 WASM（magic 不变）
        assert_eq!(&embedded[0..4], b"\0asm");

        // 可以提取出 manifest section
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
        assert_eq!(extracted, second, "第二次嵌入应替换第一次的 section");
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
            "嵌入 manifest section 后 binary_hash 应不变"
        );
    }

    #[test]
    fn embed_wasm_rejects_non_wasm() {
        let result = embed_wasm_manifest(b"ELF content", b"{}");
        assert!(result.is_err());
    }
}
