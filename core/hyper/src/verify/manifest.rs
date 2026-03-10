use sha2::{Digest, Sha256};

use crate::error::{HyperError, HyperResult};

/// ActrPackage 中嵌入的签名 manifest
///
/// 存储在二进制文件的 custom section 中（WASM custom section / ELF section / Mach-O segment）。
/// MFR 在构建期用私钥签名，Hyper 在加载前验证。
#[derive(Debug, Clone)]
pub struct PackageManifest {
    /// Actor 制造商名（`manufacturer:name:version` 中的 manufacturer）
    pub manufacturer: String,
    /// Actor 名称
    pub actr_name: String,
    /// Actor 版本
    pub version: String,
    /// 去掉签名 section 后的文件 SHA-256 哈希（32 字节）
    pub binary_hash: [u8; 32],
    /// Actor 声明的能力列表
    pub capabilities: Vec<String>,
    /// MFR 私钥对 manifest 内容（不含 signature 字段本身）的 Ed25519 签名
    pub signature: Vec<u8>,
}

impl PackageManifest {
    /// 完整的 ActrType 三段式字符串
    pub fn actr_type_str(&self) -> String {
        format!("{}:{}:{}", self.manufacturer, self.actr_name, self.version)
    }
}

// ─── WASM custom section 解析 ───────────────────────────────────────────────

/// WASM custom section 名称，用于嵌入 manifest
pub(crate) const WASM_MANIFEST_SECTION: &str = "actr-manifest";

/// 从 WASM 字节流中提取 manifest section 内容
///
/// 返回 section payload 的字节切片，不含 section 头。
/// 未找到时返回 `None`。
pub fn extract_wasm_manifest(wasm_bytes: &[u8]) -> Option<&[u8]> {
    // WASM magic + version: 8 字节
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

        // 读取 LEB128 section 大小
        let (section_size, bytes_read) = read_leb128_u32(&wasm_bytes[pos..])?;
        pos += bytes_read;

        let section_end = pos + section_size as usize;
        if section_end > wasm_bytes.len() {
            break;
        }

        // section_id == 0 是 custom section
        if section_id == 0 {
            // custom section 以名称长度 + 名称开头
            let (name_len, name_bytes_read) = read_leb128_u32(&wasm_bytes[pos..])?;
            let name_start = pos + name_bytes_read;
            let name_end = name_start + name_len as usize;

            if name_end <= section_end {
                if let Ok(name) = std::str::from_utf8(&wasm_bytes[name_start..name_end]) {
                    if name == WASM_MANIFEST_SECTION {
                        // payload 在名称之后
                        return Some(&wasm_bytes[name_end..section_end]);
                    }
                }
            }
        }

        pos = section_end;
    }

    None
}

/// 计算 WASM 文件去掉 manifest custom section 后的 SHA-256 哈希
///
/// 这是 binary_hash 的计算方式——签名时写入 section 前先算 hash，
/// 验证时先移除 section 再重算，避免循环依赖。
pub fn wasm_binary_hash_excluding_manifest(wasm_bytes: &[u8]) -> HyperResult<[u8; 32]> {
    if wasm_bytes.len() < 8 || &wasm_bytes[0..4] != b"\0asm" {
        return Err(HyperError::InvalidManifest(
            "不是有效的 WASM 文件".to_string(),
        ));
    }

    let mut hasher = Sha256::new();
    // 写入 magic + version
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
            HyperError::InvalidManifest("WASM section 大小 LEB128 解码失败".to_string())
        })?;
        let header_end = pos + bytes_read;
        pos = header_end;

        let section_end = pos + section_size as usize;
        if section_end > wasm_bytes.len() {
            return Err(HyperError::InvalidManifest(
                "WASM section 超出文件边界".to_string(),
            ));
        }

        // 跳过 manifest custom section，不纳入 hash
        if section_id == 0 {
            let (name_len, name_bytes_read) =
                read_leb128_u32(&wasm_bytes[pos..]).ok_or_else(|| {
                    HyperError::InvalidManifest("custom section 名称长度解码失败".to_string())
                })?;
            let name_start = pos + name_bytes_read;
            let name_end = name_start + name_len as usize;

            if name_end <= section_end {
                if let Ok(name) = std::str::from_utf8(&wasm_bytes[name_start..name_end]) {
                    if name == WASM_MANIFEST_SECTION {
                        // 跳过此 section，不 hash
                        pos = section_end;
                        continue;
                    }
                }
            }
        }

        // 其它所有 section 纳入 hash（包含 section id、大小、内容）
        hasher.update(&wasm_bytes[section_id_pos..section_end]);
        pos = section_end;
    }

    Ok(hasher.finalize().into())
}

pub fn is_wasm(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == b"\0asm"
}

// ─── ELF section 解析 ────────────────────────────────────────────────────────

/// ELF section 名称，用于嵌入 manifest
const ELF_MANIFEST_SECTION: &str = "actr_manifest";

/// ELF64 section header 最小大小（字节）
const ELF64_SHDR_SIZE: usize = 64;

/// 从 ELF 字节流中提取 manifest section 内容
///
/// 仅支持 ELF64 little-endian（x86_64, aarch64）。
/// 返回 `actr_manifest` section 的 payload 字节切片，未找到时返回 `None`。
pub fn extract_elf_manifest(bytes: &[u8]) -> Option<&[u8]> {
    // ELF magic: \x7fELF，ELF 文件头至少 64 字节
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return None;
    }
    // EI_CLASS（offset 4）: 2 = ELF64
    if bytes[4] != 2 {
        tracing::debug!("ELF 文件为 32-bit，暂不支持");
        return None;
    }
    // EI_DATA（offset 5）: 1 = little-endian
    if bytes[5] != 1 {
        tracing::debug!("ELF 文件为 big-endian，暂不支持");
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
        "找到 ELF actr_manifest section"
    );
    Some(&bytes[sh_off..data_end])
}

/// 计算 ELF 文件去掉 manifest section 内容（zero-fill）后的 SHA-256 哈希
///
/// zero-fill 方式：将 section 数据区替换为零字节后再计算 hash，
/// 文件大小和其他 offset 保持不变，与 binary_hash 签名约定一致。
pub fn elf_binary_hash_excluding_manifest(bytes: &[u8]) -> HyperResult<[u8; 32]> {
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return Err(HyperError::InvalidManifest(
            "不是有效的 ELF 文件".to_string(),
        ));
    }
    if bytes[4] != 2 {
        return Err(HyperError::InvalidManifest(
            "仅支持 ELF64 格式".to_string(),
        ));
    }
    if bytes[5] != 1 {
        return Err(HyperError::InvalidManifest(
            "仅支持 ELF little-endian 格式".to_string(),
        ));
    }

    let manifest_range = elf_manifest_data_range(bytes)?;

    let mut hasher = Sha256::new();
    if let Some((data_off, data_size)) = manifest_range {
        // 零填充 manifest section 数据区，其余字节正常 hash
        hasher.update(&bytes[..data_off]);
        hasher.update(vec![0u8; data_size]);
        hasher.update(&bytes[data_off + data_size..]);
    } else {
        // 未找到 manifest section，对全文件 hash（一般用于构建工具）
        tracing::debug!("ELF 中未找到 actr_manifest section，对全文件计算 hash");
        hasher.update(bytes);
    }

    Ok(hasher.finalize().into())
}

/// 查找 ELF actr_manifest section 数据区的 (file_offset, size)，失败返回错误
pub fn elf_manifest_data_range(bytes: &[u8]) -> HyperResult<Option<(usize, usize)>> {
    Ok(elf_manifest_data_range_inner(bytes))
}

/// 查找 ELF actr_manifest section 数据区的 (file_offset, size)，内部纯 Option 版本
fn elf_manifest_data_range_inner(bytes: &[u8]) -> Option<(usize, usize)> {
    // e_shoff（offset 40，8 字节）：section header table 的文件偏移
    let e_shoff = u64::from_le_bytes(bytes[40..48].try_into().ok()?) as usize;
    // e_shentsize（offset 58，2 字节）：每个 section header 的大小
    let e_shentsize = u16::from_le_bytes(bytes[58..60].try_into().ok()?) as usize;
    // e_shnum（offset 60，2 字节）：section header 条目数
    let e_shnum = u16::from_le_bytes(bytes[60..62].try_into().ok()?) as usize;
    // e_shstrndx（offset 62，2 字节）：字符串表 section 的索引
    let e_shstrndx = u16::from_le_bytes(bytes[62..64].try_into().ok()?) as usize;

    if e_shoff == 0 || e_shentsize < ELF64_SHDR_SIZE || e_shnum == 0 {
        tracing::debug!("ELF section header table 无效或不存在");
        return None;
    }
    if e_shstrndx >= e_shnum {
        tracing::warn!("ELF 字符串表索引越界: shstrndx={}", e_shstrndx);
        return None;
    }

    // 校验 section header table 整体边界
    let shdr_table_end = e_shoff.checked_add(e_shentsize.checked_mul(e_shnum)?)?;
    if shdr_table_end > bytes.len() {
        tracing::warn!("ELF section header table 超出文件边界");
        return None;
    }

    // 读取字符串表 section header（位置 = e_shoff + e_shstrndx * e_shentsize）
    let strtab_shdr_off = e_shoff + e_shstrndx * e_shentsize;
    // sh_offset 位于 section header 偏移 24，长度 8
    let strtab_off =
        u64::from_le_bytes(bytes[strtab_shdr_off + 24..strtab_shdr_off + 32].try_into().ok()?)
            as usize;
    // sh_size 位于 section header 偏移 32，长度 8
    let strtab_size =
        u64::from_le_bytes(bytes[strtab_shdr_off + 32..strtab_shdr_off + 40].try_into().ok()?)
            as usize;
    let strtab_end = strtab_off.checked_add(strtab_size)?;
    if strtab_end > bytes.len() {
        tracing::warn!("ELF 字符串表超出文件边界");
        return None;
    }
    let strtab = &bytes[strtab_off..strtab_end];

    // 遍历 section header table，查找名称为 ELF_MANIFEST_SECTION 的 section
    for i in 0..e_shnum {
        let shdr_off = e_shoff + i * e_shentsize;
        if shdr_off + ELF64_SHDR_SIZE > bytes.len() {
            break;
        }

        // sh_name：该 section 名称在字符串表中的偏移（offset 0，4 字节）
        let sh_name =
            u32::from_le_bytes(bytes[shdr_off..shdr_off + 4].try_into().ok()?) as usize;
        // sh_offset（offset 24，8 字节）：section 数据的文件偏移
        let sh_off =
            u64::from_le_bytes(bytes[shdr_off + 24..shdr_off + 32].try_into().ok()?) as usize;
        // sh_size（offset 32，8 字节）：section 数据大小
        let sh_size =
            u64::from_le_bytes(bytes[shdr_off + 32..shdr_off + 40].try_into().ok()?) as usize;

        if sh_name >= strtab.len() {
            continue;
        }
        // 从字符串表读取以 null 结尾的 section 名称
        let name_bytes = &strtab[sh_name..];
        let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
        let Ok(name) = std::str::from_utf8(&name_bytes[..name_end]) else {
            continue;
        };

        if name == ELF_MANIFEST_SECTION {
            let data_end = sh_off.checked_add(sh_size)?;
            if data_end > bytes.len() {
                tracing::warn!("ELF actr_manifest section 数据超出文件边界");
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

// ─── Mach-O section 解析 ─────────────────────────────────────────────────────

/// Mach-O __TEXT segment 名称
const MACHO_SEGMENT_NAME: &str = "__TEXT";
/// Mach-O manifest section 名称（最多 16 字节）
const MACHO_SECTION_NAME: &str = "__actr_mani";

/// Mach-O 64-bit little-endian magic（存储为 CF FA ED FE）
const MACHO_MAGIC_64_LE: u32 = 0xFEED_FACF;
/// Mach-O fat binary magic（大端 BE FE ED FA，注意与 little-endian 判断方式不同）
const MACHO_FAT_MAGIC: u32 = 0xCAFE_BABE;

/// Mach-O load command 类型：LC_SEGMENT_64
const LC_SEGMENT_64: u32 = 0x19;

/// 从 Mach-O 字节流中提取 manifest section 内容
///
/// 仅支持 64-bit little-endian（x86_64, arm64）。
/// fat binary（universal binary）不支持，返回 `None` 并记录警告。
/// 返回 `__TEXT/__actr_mani` section 的 payload 字节切片，未找到时返回 `None`。
pub fn extract_macho_manifest(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 4 {
        return None;
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().ok()?);

    // fat binary 检测（大端 magic）
    let magic_be = u32::from_be_bytes(bytes[0..4].try_into().ok()?);
    if magic_be == MACHO_FAT_MAGIC {
        tracing::warn!("检测到 Mach-O fat binary，请先使用 `lipo -thin <arch>` 提取单架构切片");
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
        "找到 Mach-O __TEXT/__actr_mani section"
    );
    Some(&bytes[data_off..data_end])
}

/// 计算 Mach-O 文件去掉 manifest section 内容（zero-fill）后的 SHA-256 哈希
///
/// fat binary 会被明确拒绝并返回含提示信息的错误。
pub fn macho_binary_hash_excluding_manifest(bytes: &[u8]) -> HyperResult<[u8; 32]> {
    if bytes.len() < 4 {
        return Err(HyperError::InvalidManifest(
            "不是有效的 Mach-O 文件（文件过短）".to_string(),
        ));
    }
    let magic_be = u32::from_be_bytes(
        bytes[0..4]
            .try_into()
            .map_err(|_| HyperError::InvalidManifest("Mach-O magic 读取失败".to_string()))?,
    );
    if magic_be == MACHO_FAT_MAGIC {
        return Err(HyperError::InvalidManifest(
            "Mach-O fat binary 暂不支持，请先使用 `lipo -thin <arch>` 提取单架构切片".to_string(),
        ));
    }

    let magic = u32::from_le_bytes(
        bytes[0..4]
            .try_into()
            .map_err(|_| HyperError::InvalidManifest("Mach-O magic 读取失败".to_string()))?,
    );
    if magic != MACHO_MAGIC_64_LE {
        return Err(HyperError::InvalidManifest(
            "不是有效的 Mach-O 64-bit little-endian 文件".to_string(),
        ));
    }

    let manifest_range = macho_manifest_data_range(bytes)?;

    let mut hasher = Sha256::new();
    if let Some((data_off, data_size)) = manifest_range {
        // 零填充 manifest section 数据区，其余字节正常 hash
        hasher.update(&bytes[..data_off]);
        hasher.update(vec![0u8; data_size]);
        hasher.update(&bytes[data_off + data_size..]);
    } else {
        tracing::debug!("Mach-O 中未找到 __actr_mani section，对全文件计算 hash");
        hasher.update(bytes);
    }

    Ok(hasher.finalize().into())
}

/// 查找 Mach-O __TEXT/__actr_mani section 的 (file_offset, size)，失败返回错误
pub fn macho_manifest_data_range(bytes: &[u8]) -> HyperResult<Option<(usize, usize)>> {
    Ok(macho_manifest_data_range_inner(bytes))
}

/// 查找 Mach-O __TEXT/__actr_mani section 的 (file_offset, size)，内部纯 Option 版本
fn macho_manifest_data_range_inner(bytes: &[u8]) -> Option<(usize, usize)> {
    // mach_header_64 布局（32 字节）：
    //   magic(4) + cputype(4) + cpusubtype(4) + filetype(4) + ncmds(4) + sizeofcmds(4) + flags(4) + reserved(4)
    if bytes.len() < 32 {
        return None;
    }

    let ncmds = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
    let sizeofcmds = u32::from_le_bytes(bytes[20..24].try_into().ok()?) as usize;

    // load commands 紧跟 mach_header_64（32 字节）之后
    let lc_start = 32usize;
    let lc_end = lc_start.checked_add(sizeofcmds)?;
    if lc_end > bytes.len() {
        tracing::warn!("Mach-O load commands 超出文件边界");
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
            // 防止死循环
            break;
        }

        if cmd == LC_SEGMENT_64 {
            // segment_command_64 布局（72 字节）：
            //   cmd(4) + cmdsize(4) + segname[16] + vmaddr(8) + vmsize(8) + fileoff(8) + filesize(8)
            //   + maxprot(4) + initprot(4) + nsects(4) + flags(4)
            // segname 在偏移 8，16 字节，以 null 填充
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
                // nsects 在 segment_command_64 偏移 64，4 字节
                let nsects = u32::from_le_bytes(bytes[pos + 64..pos + 68].try_into().ok()?) as usize;

                // section_64 紧跟 segment_command_64（72 字节）之后，每项 80 字节：
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
                        // size（offset 40，8 字节）：section 数据大小
                        let sect_size =
                            u64::from_le_bytes(bytes[sect_base + 40..sect_base + 48].try_into().ok()?)
                                as usize;
                        // offset（offset 48，4 字节）：section 数据的文件偏移
                        let sect_fileoff =
                            u32::from_le_bytes(bytes[sect_base + 48..sect_base + 52].try_into().ok()?)
                                as usize;

                        let data_end = sect_fileoff.checked_add(sect_size)?;
                        if data_end > bytes.len() {
                            tracing::warn!("Mach-O __actr_mani section 超出文件边界");
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
    // fat binary 不当作 macho 处理，交由调用方区分
    match bytes[0..4].try_into().map(u32::from_le_bytes) {
        Ok(magic) => magic == MACHO_MAGIC_64_LE,
        Err(_) => false,
    }
}

/// 解析 LEB128 编码的无符号 32 位整数
///
/// 返回 `(值, 消耗字节数)`，失败返回 `None`。
pub(crate) fn read_leb128_u32(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut result = 0u32;
    let mut shift = 0u32;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 32 {
            return None; // 溢出
        }
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None // 未终止的 LEB128
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

    // ─── ELF 测试辅助函数 ────────────────────────────────────────────────────

    /// 构建最小合法 ELF64 LE 文件字节流（含 actr_manifest section）
    ///
    /// 布局：
    ///   [0..64]   ELF header
    ///   [64..64+payload_len]  section 数据（manifest payload）
    ///   [64+payload_len..]    section header table（2 项：null + actr_manifest）
    ///                          + 字符串表数据
    ///
    /// 字符串表内容："\0actr_manifest\0"
    fn build_minimal_elf(payload: &[u8]) -> Vec<u8> {
        // 字符串表：\0 + "actr_manifest" + \0
        // "\0" 占索引 0（SHN_UNDEF name），"actr_manifest" 从索引 1 开始
        let strtab: Vec<u8> = {
            let mut v = vec![0u8]; // 索引 0：空字符串（null section 的名称）
            v.extend_from_slice(b"actr_manifest\0"); // 索引 1：manifest section 名称
            v
        };
        let strtab_name_idx: u32 = 1; // "actr_manifest" 在字符串表中的索引

        let ehdr_size: u64 = 64;
        let shdr_size: u64 = 64;
        let payload_len = payload.len() as u64;

        // section 数据紧跟 ELF header
        let manifest_data_off: u64 = ehdr_size;
        // section header table 跟在 manifest 数据之后
        let shoff: u64 = ehdr_size + payload_len;
        // 3 个 section header：null + actr_manifest + strtab
        let shnum: u16 = 3;
        let shstrndx: u16 = 2; // 字符串表是第 2 个 section（0-based）
        // 字符串表数据跟在 section header table 之后
        let strtab_off: u64 = shoff + shdr_size * shnum as u64;

        let mut buf = Vec::new();

        // ── ELF header（64 字节）──
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

        // ── manifest section 数据 ──
        buf.extend_from_slice(payload);

        // ── section header table ──
        // [0] SHN_UNDEF（null section）
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

        // [2] 字符串表 section header
        let strtab_len = strtab.len() as u64;
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_name（指向索引 0：空字符串）
        buf.extend_from_slice(&3u32.to_le_bytes()); // sh_type: SHT_STRTAB
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_flags
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_addr
        buf.extend_from_slice(&strtab_off.to_le_bytes()); // sh_offset
        buf.extend_from_slice(&strtab_len.to_le_bytes()); // sh_size
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_link
        buf.extend_from_slice(&0u32.to_le_bytes()); // sh_info
        buf.extend_from_slice(&1u64.to_le_bytes()); // sh_addralign
        buf.extend_from_slice(&0u64.to_le_bytes()); // sh_entsize

        // ── 字符串表数据 ──
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
        assert!(extract_elf_manifest(b"\x7fELF").is_none()); // 太短
    }

    #[test]
    fn elf_binary_hash_zero_fills_manifest() {
        let payload = b"manifest data";
        let elf = build_minimal_elf(payload);

        // 手工构建零填充版本验证 hash 是否一致
        let mut zeroed = elf.clone();
        // manifest 数据从偏移 64 开始，大小为 payload.len()
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

    // ─── Mach-O 测试辅助函数 ─────────────────────────────────────────────────

    /// 构建最小合法 Mach-O 64-bit LE 文件字节流（含 __TEXT/__actr_mani section）
    ///
    /// 布局：
    ///   [0..32]            mach_header_64
    ///   [32..32+lc_size]   LC_SEGMENT_64 load command（含 section_64）
    ///   [32+lc_size..]     section 数据（manifest payload）
    fn build_minimal_macho(payload: &[u8]) -> Vec<u8> {
        // mach_header_64 = 32 字节
        // segment_command_64 = 72 字节
        // section_64 = 80 字节
        // 总 load command 大小 = 72 + 80 = 152 字节
        let lc_size: u32 = 72 + 80;
        let header_size: usize = 32;
        let data_off: u32 = (header_size as u32) + lc_size;
        let payload_len = payload.len();

        let mut buf = Vec::new();

        // ── mach_header_64（32 字节）──
        buf.extend_from_slice(&MACHO_MAGIC_64_LE.to_le_bytes()); // magic
        buf.extend_from_slice(&0x0100_000cu32.to_le_bytes()); // cputype: ARM64
        buf.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
        buf.extend_from_slice(&2u32.to_le_bytes()); // filetype: MH_EXECUTE
        buf.extend_from_slice(&1u32.to_le_bytes()); // ncmds: 1 个 load command
        buf.extend_from_slice(&lc_size.to_le_bytes()); // sizeofcmds
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(buf.len(), 32);

        // ── segment_command_64（72 字节）──
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
        buf.extend_from_slice(&1u32.to_le_bytes()); // nsects: 1 个 section
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        assert_eq!(buf.len(), 32 + 72);

        // ── section_64（80 字节）──
        // sectname[16]："__actr_mani\0\0\0\0\0"
        let mut sectname = [0u8; 16];
        sectname[..11].copy_from_slice(b"__actr_mani");
        buf.extend_from_slice(&sectname);

        // segname[16]
        buf.extend_from_slice(&segname);

        buf.extend_from_slice(&0u64.to_le_bytes()); // addr
        buf.extend_from_slice(&(payload_len as u64).to_le_bytes()); // size（offset 40）
        buf.extend_from_slice(&data_off.to_le_bytes()); // offset（offset 48）
        buf.extend_from_slice(&0u32.to_le_bytes()); // align
        buf.extend_from_slice(&0u32.to_le_bytes()); // reloff
        buf.extend_from_slice(&0u32.to_le_bytes()); // nreloc
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved2
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved3
        assert_eq!(buf.len(), 32 + 72 + 80);

        // ── section 数据 ──
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
        // fat binary magic（大端：CA FE BA BE）
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

        // manifest 数据从 header_size + lc_size = 32 + 152 = 184 开始
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
        // MACHO_MAGIC_64_LE = 0xFEEDFACF，little-endian 存储为 CF FA ED FE
        let magic_le: [u8; 4] = MACHO_MAGIC_64_LE.to_le_bytes();
        let mut bytes = vec![0u8; 8];
        bytes[..4].copy_from_slice(&magic_le);
        assert!(is_macho(&bytes));
        assert!(!is_macho(b"\0asm"));
        assert!(!is_macho(b""));
    }
}
