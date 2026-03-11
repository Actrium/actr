//! `actr dev` -- Development helper commands
//!
//! Provides local development and testing stage package signing tools
//! without needing to connect to the actrix registry.
//!
//! ## Subcommands
//!
//! ```text
//! actr dev keygen [--output FILE]
//!     Generate an Ed25519 development signing key pair, saved to ~/.actr/dev-key.json by default.
//!     The public key can be configured directly in Hyper TrustMode::Development { self_signed_pubkey }.
//!
//! actr dev sign --binary FILE [--config FILE] [--key FILE] [--output FILE]
//!     Sign an Actor package (WASM/ELF/Mach-O) using the development key,
//!     embedding the manifest JSON into the corresponding custom section of the binary.
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
    /// Generate an Ed25519 development signing key pair
    Keygen(DevKeygenArgs),
    /// Sign an Actor package and embed the manifest section
    Sign(DevSignArgs),
}

#[derive(Args, Debug)]
pub struct DevKeygenArgs {
    /// Key output path (default: ~/.actr/dev-key.json)
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Force overwrite existing key
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct DevSignArgs {
    /// Target Actor binary (WASM / ELF64 / Mach-O 64)
    #[arg(long, short = 'b', value_name = "FILE")]
    pub binary: PathBuf,

    /// actr.toml path (default: actr.toml in current directory)
    #[arg(long, short = 'c', default_value = "actr.toml", value_name = "FILE")]
    pub config: PathBuf,

    /// Development signing key file (default: ~/.actr/dev-key.json)
    #[arg(long, short = 'k', value_name = "FILE")]
    pub key: Option<PathBuf>,

    /// Output file path (default: overwrite input file)
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
}

pub async fn execute(args: DevArgs) -> Result<()> {
    match args.command {
        DevCommand::Keygen(a) => execute_keygen(a),
        DevCommand::Sign(a) => execute_sign(a).await,
    }
}

// --- keygen ----------------------------------------------------------------

fn execute_keygen(args: DevKeygenArgs) -> Result<()> {
    let key_path = resolve_dev_key_path(args.output.as_deref())?;

    if key_path.exists() && !args.force {
        anyhow::bail!(
            "Key file already exists: {}\nUse --force to overwrite, or --output to specify a different path.",
            key_path.display()
        );
    }

    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying_key = signing_key.verifying_key();

    let private_b64 = base64::engine::general_purpose::STANDARD.encode(signing_key.to_bytes());
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());

    let now = chrono::Utc::now().to_rfc3339();
    let key_json = serde_json::json!({
        "private_key": private_b64,
        "public_key": public_b64,
        "created_at": now,
        "note": "Development signing key, for TrustMode::Development only, not for production use"
    });

    let json_str = serde_json::to_string_pretty(&key_json)?;
    std::fs::write(&key_path, &json_str)
        .with_context(|| format!("Failed to write key file: {}", key_path.display()))?;

    // Set file permissions (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&key_path, perms).ok();
    }

    println!("Development key generated: {}", key_path.display());
    println!();
    println!("Public key (configure in Hyper TrustMode::Development):");
    println!("  public_key: {}", public_b64);
    println!();
    println!("Hyper configuration example (TOML):");
    println!("  [hyper]");
    println!("  trust_mode = \"development\"");
    println!("  self_signed_pubkey = \"{}\"", public_b64);

    Ok(())
}

// --- sign ------------------------------------------------------------------

async fn execute_sign(args: DevSignArgs) -> Result<()> {
    // 1. Load signing key
    let key_path = resolve_dev_key_path(args.key.as_deref())?;
    let signing_key = load_signing_key(&key_path)?;
    let verifying_key = signing_key.verifying_key();
    debug!(key_path = %key_path.display(), "Development signing key loaded");

    // 2. Read actr.toml and extract manifest metadata
    let meta = load_actr_meta(&args.config)?;
    info!(
        actr_type = %format!("{}:{}:{}", meta.manufacturer, meta.name, meta.version),
        "Extracted Actor metadata from actr.toml"
    );

    // 3. Read target binary file
    let binary_bytes = std::fs::read(&args.binary)
        .with_context(|| format!("Failed to read binary file: {}", args.binary.display()))?;
    info!(
        file = %args.binary.display(),
        size = binary_bytes.len(),
        "Target binary file loaded"
    );

    // 4. Compute binary_hash (excluding existing manifest section)
    let binary_hash = compute_binary_hash(&binary_bytes).with_context(
        || "Failed to compute binary_hash; verify file format (WASM / ELF64 / Mach-O 64)",
    )?;
    let hash_hex: String = binary_hash.iter().map(|b| format!("{b:02x}")).collect();
    debug!(binary_hash = %hash_hex, "binary_hash computed");

    // 5. Build bytes to sign (identical to actr-hyper manifest_signed_bytes)
    let signed_bytes = build_signed_bytes(
        &meta.manufacturer,
        &meta.name,
        &meta.version,
        &binary_hash,
        &meta.capabilities,
    );

    // 6. Ed25519 signature
    let signature = signing_key.sign(&signed_bytes);
    debug!("Ed25519 signature computed");

    // 7. Build manifest JSON
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
    let manifest_json = serde_json::to_vec(&serde_json::json!({
        "manufacturer": meta.manufacturer,
        "actr_name": meta.name,
        "version": meta.version,
        "binary_hash": hash_hex,
        "capabilities": meta.capabilities,
        "signature": sig_b64,
    }))?;
    debug!(manifest_len = manifest_json.len(), "manifest JSON built");

    // 8. Embed manifest into binary file
    let output_path = args.output.unwrap_or_else(|| args.binary.clone());
    embed_manifest(&binary_bytes, &manifest_json, &args.binary, &output_path)?;

    // 9. Print summary
    let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());
    let fmt = detect_format(&binary_bytes);

    println!("Actor package signing completed");
    println!();
    println!(
        "  type:        {}:{}:{}",
        meta.manufacturer, meta.name, meta.version
    );
    println!("  format:      {fmt}");
    println!("  binary_hash: {}...", &hash_hex[..16]);
    println!("  signature:   {}...", &sig_b64[..16]);
    println!("  output:      {}", output_path.display());
    println!();
    println!("Development public key (Hyper TrustMode::Development self_signed_pubkey):");
    println!("  {pubkey_b64}");

    Ok(())
}

// --- Helper functions ------------------------------------------------------

/// Actor metadata extracted from actr.toml
struct ActrMeta {
    manufacturer: String,
    name: String,
    version: String,
    capabilities: Vec<String>,
}

fn load_actr_meta(config_path: &std::path::Path) -> Result<ActrMeta> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read actr.toml: {}", config_path.display()))?;

    let value: toml::Value = content
        .parse()
        .with_context(|| "Invalid actr.toml format")?;

    let pkg = value
        .get("package")
        .ok_or_else(|| anyhow::anyhow!("actr.toml missing [package] section"))?;

    let get_str = |key: &str| -> Result<String> {
        pkg.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                anyhow::anyhow!("actr.toml [package].{key} field missing or not a string")
            })
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

    Ok(ActrMeta {
        manufacturer,
        name,
        version,
        capabilities,
    })
}

fn load_signing_key(key_path: &std::path::Path) -> Result<SigningKey> {
    if !key_path.exists() {
        anyhow::bail!(
            "Development key file not found: {}\nRun `actr dev keygen` to generate a key first.",
            key_path.display()
        );
    }
    let content = std::fs::read_to_string(key_path)
        .with_context(|| format!("Failed to read key file: {}", key_path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).with_context(|| "Invalid key file JSON format")?;
    let private_b64 = json["private_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Key file missing private_key field"))?;
    let private_bytes = base64::engine::general_purpose::STANDARD
        .decode(private_b64)
        .with_context(|| "Failed to decode private_key base64")?;
    let key_arr: [u8; 32] = private_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private_key must be exactly 32 bytes (Ed25519)"))?;
    Ok(SigningKey::from_bytes(&key_arr))
}

/// Compute the binary_hash of a binary file (excluding existing manifest section)
fn compute_binary_hash(bytes: &[u8]) -> Result<[u8; 32]> {
    use actr_hyper::verify::manifest::{
        elf_binary_hash_excluding_manifest, is_elf, is_macho, is_wasm,
        macho_binary_hash_excluding_manifest, wasm_binary_hash_excluding_manifest,
    };
    if is_wasm(bytes) {
        Ok(wasm_binary_hash_excluding_manifest(bytes)
            .map_err(|e| anyhow::anyhow!("WASM binary_hash computation failed: {e}"))?)
    } else if is_elf(bytes) {
        Ok(elf_binary_hash_excluding_manifest(bytes)
            .map_err(|e| anyhow::anyhow!("ELF binary_hash computation failed: {e}"))?)
    } else if is_macho(bytes) {
        Ok(macho_binary_hash_excluding_manifest(bytes)
            .map_err(|e| anyhow::anyhow!("Mach-O binary_hash computation failed: {e}"))?)
    } else {
        anyhow::bail!(
            "Unsupported file format; only WASM / ELF64 LE / Mach-O 64-bit LE are supported"
        )
    }
}

/// Build bytes to sign (identical to actr-hyper verify/mod.rs manifest_signed_bytes)
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

/// Embed manifest JSON into a binary file and write to output_path
fn embed_manifest(
    bytes: &[u8],
    manifest_json: &[u8],
    input_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<()> {
    use actr_hyper::verify::embed::{
        embed_elf_manifest, embed_macho_manifest, embed_wasm_manifest,
    };
    use actr_hyper::verify::manifest::{is_elf, is_macho, is_wasm};

    if is_wasm(bytes) {
        let embedded = embed_wasm_manifest(bytes, manifest_json)
            .map_err(|e| anyhow::anyhow!("WASM manifest embedding failed: {e}"))?;
        std::fs::write(output_path, &embedded)
            .with_context(|| format!("Failed to write output file: {}", output_path.display()))?;
    } else if is_elf(bytes) {
        // ELF: via objcopy subprocess, in-place or write to new file
        if input_path != output_path {
            std::fs::copy(input_path, output_path).with_context(|| "Failed to copy ELF file")?;
        }
        embed_elf_manifest(input_path, output_path, manifest_json)
            .map_err(|e| anyhow::anyhow!("ELF manifest embedding failed: {e}\nHint: binutils required (Ubuntu: apt install binutils)"))?;
    } else if is_macho(bytes) {
        if input_path != output_path {
            std::fs::copy(input_path, output_path).with_context(|| "Failed to copy Mach-O file")?;
        }
        embed_macho_manifest(input_path, output_path, manifest_json)
            .map_err(|e| anyhow::anyhow!("Mach-O manifest embedding failed: {e}\nHint: LLVM toolchain required (macOS: brew install llvm)"))?;
    } else {
        warn!("Unrecognized file format, skipping embedding");
        anyhow::bail!("Unsupported file format");
    }
    Ok(())
}

fn detect_format(bytes: &[u8]) -> &'static str {
    use actr_hyper::verify::manifest::{is_elf, is_macho, is_wasm};
    if is_wasm(bytes) {
        "WASM"
    } else if is_elf(bytes) {
        "ELF64 LE"
    } else if is_macho(bytes) {
        "Mach-O 64-bit LE"
    } else {
        "unknown"
    }
}

fn resolve_dev_key_path(custom: Option<&std::path::Path>) -> Result<PathBuf> {
    if let Some(p) = custom {
        return Ok(p.to_path_buf());
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
    Ok(home.join(".actr").join("dev-key.json"))
}
