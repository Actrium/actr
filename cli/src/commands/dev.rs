//! `actr dev` — 开发辅助命令
//!
//! 提供本地开发、测试阶段的包签名工具，无需连接 actrix 注册中心。
//!
//! ## 子命令
//!
//! ```text
//! actr dev keygen [--output FILE]
//!     生成 Ed25519 开发签名密钥对，默认保存到 ~/.actr/dev-key.json。
//!     公钥可直接配置到 Hyper TrustMode::Development { self_signed_pubkey }。
//!
//! actr dev sign --binary FILE [--config FILE] [--key FILE] [--output FILE]
//!     使用开发密钥对 Actor 包（WASM/ELF/Mach-O）进行签名，
//!     将 manifest JSON 嵌入二进制文件对应的 custom section。
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::Engine;
use clap::{Args, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use tracing::{debug, info, warn};

#[derive(Args, Debug)]
pub struct DevArgs {
    #[command(subcommand)]
    pub command: DevCommand,
}

#[derive(Subcommand, Debug)]
pub enum DevCommand {
    /// 生成 Ed25519 开发签名密钥对
    Keygen(DevKeygenArgs),
    /// 为 Actor 包签名并嵌入 manifest section
    Sign(DevSignArgs),
}

#[derive(Args, Debug)]
pub struct DevKeygenArgs {
    /// 密钥输出路径（默认：~/.actr/dev-key.json）
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// 强制覆盖已有密钥
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct DevSignArgs {
    /// 目标 Actor 二进制文件（WASM / ELF64 / Mach-O 64）
    #[arg(long, short = 'b', value_name = "FILE")]
    pub binary: PathBuf,

    /// actr.toml 路径（默认：当前目录 actr.toml）
    #[arg(long, short = 'c', default_value = "actr.toml", value_name = "FILE")]
    pub config: PathBuf,

    /// 开发签名密钥文件（默认：~/.actr/dev-key.json）
    #[arg(long, short = 'k', value_name = "FILE")]
    pub key: Option<PathBuf>,

    /// 输出文件路径（默认：覆盖输入文件）
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
}

pub async fn execute(args: DevArgs) -> Result<()> {
    match args.command {
        DevCommand::Keygen(a) => execute_keygen(a),
        DevCommand::Sign(a) => execute_sign(a).await,
    }
}

// ─── keygen ──────────────────────────────────────────────────────────────────

fn execute_keygen(args: DevKeygenArgs) -> Result<()> {
    let key_path = resolve_dev_key_path(args.output.as_deref())?;

    if key_path.exists() && !args.force {
        anyhow::bail!(
            "密钥文件已存在：{}\n使用 --force 覆盖，或用 --output 指定其他路径。",
            key_path.display()
        );
    }

    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败：{}", parent.display()))?;
    }

    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying_key = signing_key.verifying_key();

    let private_b64 = base64::engine::general_purpose::STANDARD
        .encode(signing_key.to_bytes());
    let public_b64 = base64::engine::general_purpose::STANDARD
        .encode(verifying_key.to_bytes());

    let now = chrono::Utc::now().to_rfc3339();
    let key_json = serde_json::json!({
        "private_key": private_b64,
        "public_key": public_b64,
        "created_at": now,
        "note": "开发签名密钥，仅用于 TrustMode::Development，不可用于生产环境"
    });

    let json_str = serde_json::to_string_pretty(&key_json)?;
    std::fs::write(&key_path, &json_str)
        .with_context(|| format!("写入密钥文件失败：{}", key_path.display()))?;

    // 设置文件权限（仅所有者可读写）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&key_path, perms).ok();
    }

    println!("开发密钥已生成：{}", key_path.display());
    println!();
    println!("公钥（配置到 Hyper TrustMode::Development）：");
    println!("  public_key: {}", public_b64);
    println!();
    println!("Hyper 配置示例（TOML）：");
    println!("  [hyper]");
    println!("  trust_mode = \"development\"");
    println!("  self_signed_pubkey = \"{}\"", public_b64);

    Ok(())
}

// ─── sign ─────────────────────────────────────────────────────────────────────

async fn execute_sign(args: DevSignArgs) -> Result<()> {
    // 1. 加载签名密钥
    let key_path = resolve_dev_key_path(args.key.as_deref())?;
    let signing_key = load_signing_key(&key_path)?;
    let verifying_key = signing_key.verifying_key();
    debug!(key_path = %key_path.display(), "开发签名密钥已加载");

    // 2. 读取 actr.toml，提取 manifest 元数据
    let meta = load_actr_meta(&args.config)?;
    info!(
        actr_type = %format!("{}:{}:{}", meta.manufacturer, meta.name, meta.version),
        "从 actr.toml 提取 Actor 元数据"
    );

    // 3. 读取目标二进制文件
    let binary_bytes = std::fs::read(&args.binary)
        .with_context(|| format!("读取二进制文件失败：{}", args.binary.display()))?;
    info!(
        file = %args.binary.display(),
        size = binary_bytes.len(),
        "目标二进制文件已读取"
    );

    // 4. 计算 binary_hash（排除已有 manifest section）
    let binary_hash = compute_binary_hash(&binary_bytes)
        .with_context(|| "计算 binary_hash 失败，请确认文件格式（WASM / ELF64 / Mach-O 64）")?;
    let hash_hex: String = binary_hash.iter().map(|b| format!("{b:02x}")).collect();
    debug!(binary_hash = %hash_hex, "binary_hash 计算完成");

    // 5. 构建待签名字节（与 actr-hyper manifest_signed_bytes 完全一致）
    let signed_bytes = build_signed_bytes(
        &meta.manufacturer,
        &meta.name,
        &meta.version,
        &binary_hash,
        &meta.capabilities,
    );

    // 6. Ed25519 签名
    let signature = signing_key.sign(&signed_bytes);
    debug!("Ed25519 签名计算完成");

    // 7. 构建 manifest JSON
    let sig_b64 = base64::engine::general_purpose::STANDARD
        .encode(signature.to_bytes());
    let manifest_json = serde_json::to_vec(&serde_json::json!({
        "manufacturer": meta.manufacturer,
        "actr_name": meta.name,
        "version": meta.version,
        "binary_hash": hash_hex,
        "capabilities": meta.capabilities,
        "signature": sig_b64,
    }))?;
    debug!(manifest_len = manifest_json.len(), "manifest JSON 构建完成");

    // 8. 嵌入 manifest 到二进制文件
    let output_path = args.output.unwrap_or_else(|| args.binary.clone());
    embed_manifest(&binary_bytes, &manifest_json, &args.binary, &output_path)?;

    // 9. 打印摘要
    let pubkey_b64 = base64::engine::general_purpose::STANDARD
        .encode(verifying_key.to_bytes());
    let fmt = detect_format(&binary_bytes);

    println!("Actor 包签名完成");
    println!();
    println!("  类型:        {}:{}:{}", meta.manufacturer, meta.name, meta.version);
    println!("  格式:        {fmt}");
    println!("  binary_hash: {}...", &hash_hex[..16]);
    println!("  签名:        {}...", &sig_b64[..16]);
    println!("  输出:        {}", output_path.display());
    println!();
    println!("开发公钥（Hyper TrustMode::Development self_signed_pubkey）：");
    println!("  {pubkey_b64}");

    Ok(())
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 从 actr.toml 提取的 Actor 元数据
struct ActrMeta {
    manufacturer: String,
    name: String,
    version: String,
    capabilities: Vec<String>,
}

fn load_actr_meta(config_path: &std::path::Path) -> Result<ActrMeta> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("读取 actr.toml 失败：{}", config_path.display()))?;

    let value: toml::Value = content.parse()
        .with_context(|| "actr.toml 格式无效")?;

    let pkg = value.get("package").ok_or_else(|| {
        anyhow::anyhow!("actr.toml 缺少 [package] section")
    })?;

    let get_str = |key: &str| -> Result<String> {
        pkg.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("actr.toml [package].{key} 字段缺失或非字符串"))
    };

    let manufacturer = get_str("manufacturer")?;
    let name = get_str("name")?;
    let version = get_str("version")?;

    let capabilities = value
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(ActrMeta { manufacturer, name, version, capabilities })
}

fn load_signing_key(key_path: &std::path::Path) -> Result<SigningKey> {
    if !key_path.exists() {
        anyhow::bail!(
            "开发密钥文件不存在：{}\n请先运行 `actr dev keygen` 生成密钥。",
            key_path.display()
        );
    }
    let content = std::fs::read_to_string(key_path)
        .with_context(|| format!("读取密钥文件失败：{}", key_path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| "密钥文件 JSON 格式无效")?;
    let private_b64 = json["private_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("密钥文件缺少 private_key 字段"))?;
    let private_bytes = base64::engine::general_purpose::STANDARD
        .decode(private_b64)
        .with_context(|| "private_key base64 解码失败")?;
    let key_arr: [u8; 32] = private_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private_key 必须为 32 字节（Ed25519）"))?;
    Ok(SigningKey::from_bytes(&key_arr))
}

/// 计算二进制文件的 binary_hash（排除已有 manifest section）
fn compute_binary_hash(bytes: &[u8]) -> Result<[u8; 32]> {
    use actr_hyper::verify::manifest::{
        is_wasm, is_elf, is_macho,
        wasm_binary_hash_excluding_manifest,
        elf_binary_hash_excluding_manifest,
        macho_binary_hash_excluding_manifest,
    };
    if is_wasm(bytes) {
        Ok(wasm_binary_hash_excluding_manifest(bytes)
            .map_err(|e| anyhow::anyhow!("WASM binary_hash 计算失败: {e}"))?)
    } else if is_elf(bytes) {
        Ok(elf_binary_hash_excluding_manifest(bytes)
            .map_err(|e| anyhow::anyhow!("ELF binary_hash 计算失败: {e}"))?)
    } else if is_macho(bytes) {
        Ok(macho_binary_hash_excluding_manifest(bytes)
            .map_err(|e| anyhow::anyhow!("Mach-O binary_hash 计算失败: {e}"))?)
    } else {
        anyhow::bail!("不支持的文件格式，仅支持 WASM / ELF64 LE / Mach-O 64-bit LE")
    }
}

/// 构建待签名字节（与 actr-hyper verify/mod.rs manifest_signed_bytes 完全一致）
fn build_signed_bytes(
    manufacturer: &str,
    actr_name: &str,
    version: &str,
    binary_hash: &[u8; 32],
    capabilities: &[String],
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(manufacturer.as_bytes());
    buf.push(0);
    buf.extend_from_slice(actr_name.as_bytes());
    buf.push(0);
    buf.extend_from_slice(version.as_bytes());
    buf.push(0);
    buf.extend_from_slice(binary_hash);
    buf.push(0);
    for cap in capabilities {
        buf.extend_from_slice(cap.as_bytes());
        buf.push(0);
    }
    buf
}

/// 将 manifest JSON 嵌入二进制文件并写入 output_path
fn embed_manifest(
    bytes: &[u8],
    manifest_json: &[u8],
    input_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<()> {
    use actr_hyper::verify::manifest::{is_wasm, is_elf, is_macho};
    use actr_hyper::verify::embed::{embed_wasm_manifest, embed_elf_manifest, embed_macho_manifest};

    if is_wasm(bytes) {
        let embedded = embed_wasm_manifest(bytes, manifest_json)
            .map_err(|e| anyhow::anyhow!("WASM manifest 嵌入失败: {e}"))?;
        std::fs::write(output_path, &embedded)
            .with_context(|| format!("写入输出文件失败：{}", output_path.display()))?;
    } else if is_elf(bytes) {
        // ELF：通过 objcopy 子进程，in-place 或写入新文件
        if input_path != output_path {
            std::fs::copy(input_path, output_path)
                .with_context(|| "复制 ELF 文件失败")?;
        }
        embed_elf_manifest(input_path, output_path, manifest_json)
            .map_err(|e| anyhow::anyhow!("ELF manifest 嵌入失败: {e}\n提示：需要安装 binutils（Ubuntu: apt install binutils）"))?;
    } else if is_macho(bytes) {
        if input_path != output_path {
            std::fs::copy(input_path, output_path)
                .with_context(|| "复制 Mach-O 文件失败")?;
        }
        embed_macho_manifest(input_path, output_path, manifest_json)
            .map_err(|e| anyhow::anyhow!("Mach-O manifest 嵌入失败: {e}\n提示：需要安装 LLVM 工具链（macOS: brew install llvm）"))?;
    } else {
        warn!("无法识别的文件格式，跳过嵌入");
        anyhow::bail!("不支持的文件格式");
    }
    Ok(())
}

fn detect_format(bytes: &[u8]) -> &'static str {
    use actr_hyper::verify::manifest::{is_wasm, is_elf, is_macho};
    if is_wasm(bytes) { "WASM" }
    else if is_elf(bytes) { "ELF64 LE" }
    else if is_macho(bytes) { "Mach-O 64-bit LE" }
    else { "unknown" }
}

fn resolve_dev_key_path(custom: Option<&std::path::Path>) -> Result<PathBuf> {
    if let Some(p) = custom {
        return Ok(p.to_path_buf());
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("无法获取用户主目录"))?;
    Ok(home.join(".actr").join("dev-key.json"))
}
