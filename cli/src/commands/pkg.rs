//! `actr pkg` — Package management commands
//!
//! ## Subcommands
//!
//! ```text
//! actr pkg build    --binary FILE [--config actr.toml] [--key FILE] [--output FILE]
//! actr pkg sign     --keychain FILE [--package FILE]
//! actr pkg verify   --package FILE [--pubkey FILE]
//! actr pkg keygen   [--output FILE] [--force]
//! actr pkg publish  --package FILE --keychain FILE --endpoint URL
//! ```

use std::path::PathBuf;

use actr_protocol::{ActrType, ActrTypeExt};
use anyhow::{Context, Result};
use base64::Engine;
use clap::{Args, Subcommand};
use ed25519_dalek::SigningKey;

#[derive(Args, Debug)]
pub struct PkgArgs {
    #[command(subcommand)]
    pub command: PkgCommand,
}

#[derive(Subcommand, Debug)]
pub enum PkgCommand {
    /// Build an .actr package from binary and config
    Build(PkgBuildArgs),
    /// Sign an .actr package with MFR private key
    Sign(PkgSignArgs),
    /// Verify an .actr package
    Verify(PkgVerifyArgs),
    /// Generate an Ed25519 signing key pair
    Keygen(PkgKeygenArgs),
    /// Publish an .actr package to the Actrix MFR registry
    Publish(PkgPublishArgs),
}

#[derive(Args, Debug)]
pub struct PkgBuildArgs {
    /// Target actor binary (WASM / native)
    #[arg(long, short = 'b', value_name = "FILE")]
    pub binary: PathBuf,

    /// actr.toml config path
    #[arg(long, short = 'c', default_value = "actr.toml", value_name = "FILE")]
    pub config: PathBuf,

    /// Signing key file (default: ~/.actr/dev-key.json)
    #[arg(long, short = 'k', value_name = "FILE")]
    pub key: Option<PathBuf>,

    /// Output .actr file path
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Target platform (e.g., wasm32-wasip1, x86_64-unknown-linux-gnu)
    #[arg(long, short = 't', default_value = "wasm32-wasip1")]
    pub target: String,
}

#[derive(Args, Debug)]
pub struct PkgSignArgs {
    /// Path to MFR keychain JSON file
    #[arg(long, short = 'k', value_name = "FILE")]
    pub keychain: PathBuf,

    /// .actr package file to sign
    #[arg(long, short = 'p', value_name = "FILE")]
    pub package: Option<PathBuf>,

    /// Path to actr.toml (used if --package not specified)
    #[arg(long, short = 'c', default_value = "actr.toml", value_name = "FILE")]
    pub config: PathBuf,

    /// Path to actor binary (optional, for hash computation)
    #[arg(long, short = 'b', value_name = "FILE")]
    pub binary: Option<PathBuf>,

    /// Output signature file
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct PkgVerifyArgs {
    /// .actr package file to verify
    #[arg(long, short = 'p', value_name = "FILE")]
    pub package: PathBuf,

    /// Public key file (default: extract from package or use dev-key)
    #[arg(long, value_name = "FILE")]
    pub pubkey: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct PkgKeygenArgs {
    /// Key output path (default: ~/.actr/dev-key.json)
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Force overwrite existing key
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct PkgPublishArgs {
    /// .actr package file to publish
    #[arg(long, short = 'p', value_name = "FILE")]
    pub package: PathBuf,

    /// Path to MFR keychain JSON file (contains private_key for signing)
    #[arg(long, short = 'k', value_name = "FILE")]
    pub keychain: PathBuf,

    /// Actrix MFR endpoint URL (e.g., http://localhost:8081).
    /// If omitted, reads from [system.ais_endpoint].url in actr.toml.
    #[arg(long, short = 'e', value_name = "URL")]
    pub endpoint: Option<String>,

    /// Path to actr.toml (used to resolve default endpoint).
    /// Defaults to ./actr.toml in the current directory.
    #[arg(long, short = 'c', value_name = "FILE", default_value = "actr.toml")]
    pub config: PathBuf,
}

pub async fn execute(args: PkgArgs) -> Result<()> {
    match args.command {
        PkgCommand::Build(a) => execute_build(a).await,
        PkgCommand::Sign(a) => execute_sign(a).await,
        PkgCommand::Verify(a) => execute_verify(a).await,
        PkgCommand::Keygen(a) => execute_keygen(a),
        PkgCommand::Publish(a) => execute_publish(a).await,
    }
}

// --- keygen (moved from dev.rs, identical logic) ---

fn execute_keygen(args: PkgKeygenArgs) -> Result<()> {
    let key_path = resolve_key_path(args.output.as_deref())?;

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

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&key_path, perms).ok();
    }

    println!("Key pair generated: {}", key_path.display());
    println!();
    println!("Public key (for Hyper TrustMode::Development):");
    println!("  {}", public_b64);
    println!();
    println!("Hyper configuration example (TOML):");
    println!("  [hyper]");
    println!("  trust_mode = \"development\"");
    println!("  self_signed_pubkey = \"{}\"", public_b64);

    Ok(())
}

// --- build (.actr package creation) ---

async fn execute_build(args: PkgBuildArgs) -> Result<()> {
    use sha2::{Digest, Sha256};

    // 1. Load signing key
    let key_path = resolve_key_path(args.key.as_deref())?;
    let signing_key = load_signing_key(&key_path)?;
    let verifying_key = signing_key.verifying_key();
    tracing::debug!(key_path = %key_path.display(), "signing key loaded");

    // 2. Read actr.toml for metadata
    let config_bytes = std::fs::read(&args.config)
        .with_context(|| format!("Failed to read config: {}", args.config.display()))?;
    let config_value: toml::Value =
        toml::from_slice(&config_bytes).with_context(|| "Invalid actr.toml")?;
    let pkg = config_value
        .get("package")
        .ok_or_else(|| anyhow::anyhow!("actr.toml missing [package] section"))?;

    // Support both [package.actr_type].{manufacturer,name,version} (standard actr.toml)
    // and flat [package].{manufacturer,name,version} (legacy format)
    let actr_type_table = pkg.get("actr_type");
    let get_str = |key: &str| -> Result<String> {
        actr_type_table
            .and_then(|t| t.get(key))
            .or_else(|| pkg.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("actr.toml [package.actr_type].{key} missing"))
    };

    let manufacturer = get_str("manufacturer")?;
    let name = get_str("name")?;
    let version = get_str("version")?;
    let actr_type = ActrType {
        manufacturer: manufacturer.clone(),
        name: name.clone(),
        version: version.clone(),
    };

    // 3. Read binary
    let binary_bytes = std::fs::read(&args.binary)
        .with_context(|| format!("Failed to read binary: {}", args.binary.display()))?;

    tracing::info!(
        actr_type = %actr_type.to_string_repr(),
        binary_size = binary_bytes.len(),
        "building .actr package"
    );

    // 4. Create manifest
    let manifest = actr_pack::PackageManifest {
        manufacturer: manufacturer.clone(),
        name: name.clone(),
        version: version.clone(),
        binary: actr_pack::BinaryEntry {
            path: "bin/actor.wasm".to_string(),
            target: args.target.clone(),
            hash: String::new(),
            size: None,
        },
        signature_algorithm: "ed25519".to_string(),
        resources: vec![],
        metadata: actr_pack::ManifestMetadata {
            description: pkg
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            license: pkg
                .get("license")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        },
    };

    // 5. Pack
    let opts = actr_pack::PackOptions {
        manifest,
        binary_bytes: binary_bytes.clone(),
        resources: vec![],
        signing_key: signing_key.clone(),
    };
    let package_bytes = actr_pack::pack(&opts)?;

    // 6. Write output
    let output_path = args.output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "{}-{}-{}-{}.actr",
            manufacturer, name, version, args.target
        ))
    });
    std::fs::write(&output_path, &package_bytes)
        .with_context(|| format!("Failed to write package: {}", output_path.display()))?;

    // 7. Summary
    let mut hasher = Sha256::new();
    hasher.update(&binary_bytes);
    let hash_hex = hex::encode(hasher.finalize());

    let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());

    println!("Package built successfully");
    println!();
    println!("  type:        {}:{}:{}", manufacturer, name, version);
    println!("  target:      {}", args.target);
    println!("  binary_hash: {}...", &hash_hex[..16]);
    println!("  output:      {}", output_path.display());
    println!("  size:        {} bytes", package_bytes.len());
    println!();
    println!("Public key (for verification):");
    println!("  {pubkey_b64}");

    Ok(())
}

// --- sign (MFR signing, adapted from old pkg sign) ---

async fn execute_sign(args: PkgSignArgs) -> Result<()> {
    use ed25519_dalek::{Signer, SigningKey as DalekSigningKey};
    use sha2::{Digest, Sha256};
    use std::io::Write;

    // 1. Read keychain JSON
    tracing::debug!("reading keychain file: {:?}", args.keychain);
    let keychain_content = std::fs::read_to_string(&args.keychain)
        .map_err(|e| anyhow::anyhow!("failed to read keychain file {:?}: {}", args.keychain, e))?;
    let keychain: serde_json::Value = serde_json::from_str(&keychain_content)
        .map_err(|e| anyhow::anyhow!("invalid keychain JSON: {}", e))?;
    let private_key_b64 = keychain["private_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("keychain missing 'private_key' field"))?;

    // 2. Decode private key
    let private_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(private_key_b64)
        .map_err(|e| anyhow::anyhow!("invalid private key base64: {}", e))?;
    let key_array: [u8; 32] = private_key_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private key must be exactly 32 bytes"))?;
    let signing_key = DalekSigningKey::from_bytes(&key_array);

    // 3. Read actr.toml
    let config_path = &args.config;
    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "config file not found: {}",
            config_path.display()
        ));
    }
    let config_content = std::fs::read_to_string(config_path)?;

    // 4. If binary specified, compute sha256
    let final_config = if let Some(binary_path) = &args.binary {
        let binary_data = std::fs::read(binary_path)?;
        let mut hasher = Sha256::new();
        hasher.update(&binary_data);
        let hash_hex = hex::encode(hasher.finalize());
        let binary_hash = format!("sha256:{}", hash_hex);
        println!("binary_hash = \"{}\"", binary_hash);
        // Insert/update binary_hash in config
        insert_binary_hash(&config_content, &binary_hash)?
    } else {
        config_content
    };

    // 5. Sign
    let manifest_bytes = final_config.as_bytes();
    let signature = signing_key.sign(manifest_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

    // 6. Write signature file
    let sig_path = args.output.unwrap_or_else(|| {
        let mut p = args.config.clone();
        let new_name = format!(
            "{}.sig",
            p.file_name().unwrap_or_default().to_string_lossy()
        );
        p.set_file_name(new_name);
        p
    });
    {
        let mut f = std::fs::File::create(&sig_path)?;
        writeln!(f, "{}", sig_b64)?;
    }

    println!("Package signed successfully");
    println!("  signature: {}...", &sig_b64[..16]);
    println!("  sig file:  {}", sig_path.display());

    Ok(())
}

// --- verify ---

async fn execute_verify(args: PkgVerifyArgs) -> Result<()> {
    // 1. Read package
    let package_bytes = std::fs::read(&args.package)
        .with_context(|| format!("Failed to read package: {}", args.package.display()))?;

    // 2. Read public key
    let pubkey = if let Some(pubkey_path) = &args.pubkey {
        load_verifying_key(pubkey_path)?
    } else {
        // Try default dev-key location
        let key_path = resolve_key_path(None)?;
        load_verifying_key_from_dev_key(&key_path)?
    };

    // 3. Verify
    let verified = actr_pack::verify(&package_bytes, &pubkey)?;

    println!("Package verification passed");
    println!();
    println!("  type:        {}", verified.manifest.actr_type_str());
    println!("  binary:      {}", verified.manifest.binary.path);
    println!("  binary_hash: {}...", &verified.manifest.binary.hash[..16]);
    println!("  target:      {}", verified.manifest.binary.target);
    if !verified.manifest.resources.is_empty() {
        println!("  resources:   {}", verified.manifest.resources.len());
    }

    Ok(())
}

// --- publish (register package to MFR registry) ---

async fn execute_publish(args: PkgPublishArgs) -> Result<()> {
    use ed25519_dalek::{Signer, SigningKey as DalekSigningKey};

    // 1. Read .actr package
    tracing::debug!("reading .actr package: {:?}", args.package);
    let package_bytes = std::fs::read(&args.package)
        .with_context(|| format!("Failed to read package: {}", args.package.display()))?;

    // 2. Extract raw manifest TOML from the .actr ZIP
    let manifest_str = actr_pack::read_manifest_raw(&package_bytes)
        .with_context(|| "Failed to read manifest from .actr package")?;
    let manifest = actr_pack::PackageManifest::from_toml(&manifest_str)
        .with_context(|| "Failed to parse manifest TOML")?;

    println!("📦 Publishing package: {}", manifest.actr_type_str());
    println!("   manufacturer: {}", manifest.manufacturer);
    println!("   name:         {}", manifest.name);
    println!("   version:      {}", manifest.version);

    // 3. Load MFR keychain and extract private key
    tracing::debug!("loading keychain: {:?}", args.keychain);
    let keychain_content = std::fs::read_to_string(&args.keychain)
        .with_context(|| format!("Failed to read keychain: {}", args.keychain.display()))?;
    let keychain: serde_json::Value =
        serde_json::from_str(&keychain_content).with_context(|| "Invalid keychain JSON")?;
    let private_key_b64 = keychain["private_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Keychain missing 'private_key' field"))?;

    let private_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(private_key_b64)
        .with_context(|| "Invalid private key base64 encoding")?;
    let key_array: [u8; 32] = private_key_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("Private key must be exactly 32 bytes"))?;
    let signing_key = DalekSigningKey::from_bytes(&key_array);

    // 4. Sign the manifest TOML bytes with MFR private key
    let signature = signing_key.sign(manifest_str.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
    println!("🔐 Manifest signed with MFR key");

    // 5. Resolve endpoint: --endpoint flag > actr.toml [system.ais_endpoint].url
    let endpoint = if let Some(ep) = args.endpoint {
        ep
    } else {
        let config_content = std::fs::read_to_string(&args.config)
            .with_context(|| format!("Failed to read config: {}", args.config.display()))?;
        let config_value: toml::Value =
            toml::from_str(&config_content).with_context(|| "Invalid actr.toml")?;
        config_value
            .get("system")
            .and_then(|s| s.get("ais_endpoint"))
            .and_then(|a| a.get("url"))
            .and_then(|u| u.as_str())
            .map(|u| {
                // ais_endpoint url may include /ais path, strip it for base endpoint
                u.trim_end_matches("/ais").to_string()
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No --endpoint provided and [system.ais_endpoint].url not found in {}",
                    args.config.display()
                )
            })?
    };

    // 6. POST /mfr/pkg/publish
    let publish_url = format!("{}/mfr/pkg/publish", endpoint.trim_end_matches('/'));
    println!("📡 Publishing to: {}", publish_url);

    let client = reqwest::Client::new();
    let resp = client
        .post(&publish_url)
        .json(&serde_json::json!({
            "manufacturer": manifest.manufacturer,
            "name": manifest.name,
            "version": manifest.version,
            "target": manifest.binary.target,
            "manifest": manifest_str,
            "signature": sig_b64,
        }))
        .send()
        .await
        .with_context(|| format!("Failed to connect to MFR endpoint: {}", publish_url))?;

    // 6. Handle response
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        eprintln!("❌ Publish failed (HTTP {})", status);
        eprintln!("   Response: {}", body);
        anyhow::bail!("MFR publish failed with status {}: {}", status, body);
    }

    // Parse response to show type_str
    if let Ok(result) = serde_json::from_str::<serde_json::Value>(&body) {
        let type_str = result["type_str"].as_str().unwrap_or("unknown");
        let pkg_id = result["id"].as_i64().unwrap_or(0);
        println!();
        println!("✅ Package published successfully!");
        println!("   type_str:  {}", type_str);
        println!("   pkg_id:    {}", pkg_id);
        println!("   status:    active");
    } else {
        println!();
        println!("✅ Package published successfully!");
        println!("   Response: {}", body);
    }

    Ok(())
}

// --- Helpers ---

fn resolve_key_path(custom: Option<&std::path::Path>) -> Result<PathBuf> {
    if let Some(p) = custom {
        return Ok(p.to_path_buf());
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
    Ok(home.join(".actr").join("dev-key.json"))
}

fn load_signing_key(key_path: &std::path::Path) -> Result<SigningKey> {
    if !key_path.exists() {
        anyhow::bail!(
            "Key file not found: {}\nRun `actr pkg keygen` to generate a key first.",
            key_path.display()
        );
    }
    let content = std::fs::read_to_string(key_path)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;
    let private_b64 = json["private_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Key file missing private_key field"))?;
    let private_bytes = base64::engine::general_purpose::STANDARD.decode(private_b64)?;
    let key_arr: [u8; 32] = private_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private_key must be exactly 32 bytes"))?;
    Ok(SigningKey::from_bytes(&key_arr))
}

fn load_verifying_key(path: &std::path::Path) -> Result<ed25519_dalek::VerifyingKey> {
    let content = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;
    let public_b64 = json["public_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Key file missing public_key field"))?;
    let public_bytes = base64::engine::general_purpose::STANDARD.decode(public_b64)?;
    let key_arr: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("public_key must be exactly 32 bytes"))?;
    ed25519_dalek::VerifyingKey::from_bytes(&key_arr)
        .map_err(|e| anyhow::anyhow!("Invalid public key: {e}"))
}

fn load_verifying_key_from_dev_key(path: &std::path::Path) -> Result<ed25519_dalek::VerifyingKey> {
    if !path.exists() {
        anyhow::bail!(
            "No key file found at {}. Specify --pubkey or run `actr pkg keygen` first.",
            path.display()
        );
    }
    load_verifying_key(path)
}

/// Insert or update `binary_hash` in actr.toml (same logic as old pkg.rs)
fn insert_binary_hash(content: &str, binary_hash: &str) -> Result<String> {
    let hash_line = format!("binary_hash = \"{}\"", binary_hash);

    if content.contains("binary_hash") {
        let replaced = content
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("binary_hash") {
                    hash_line.clone()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let trailing = if content.ends_with('\n') { "\n" } else { "" };
        return Ok(format!("{}{}", replaced, trailing));
    }

    let mut result = String::with_capacity(content.len() + hash_line.len() + 2);
    let mut in_package = false;
    let mut inserted = false;
    for line in content.lines() {
        if !inserted {
            let trimmed = line.trim();
            if trimmed == "[package]" {
                in_package = true;
            } else if in_package && trimmed.starts_with('[') && trimmed.ends_with(']') {
                result.push_str(&hash_line);
                result.push('\n');
                inserted = true;
                in_package = false;
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    if !inserted {
        result.push_str(&hash_line);
        result.push('\n');
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_binary_hash_new() {
        let content = "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n\n[dependencies]\n";
        let result = insert_binary_hash(content, "sha256:abc123").unwrap();
        assert!(result.contains("binary_hash = \"sha256:abc123\""));
        // Should be before [dependencies]
        let hash_pos = result.find("binary_hash").unwrap();
        let dep_pos = result.find("[dependencies]").unwrap();
        assert!(hash_pos < dep_pos);
    }

    #[test]
    fn test_insert_binary_hash_replace() {
        let content = "[package]\nname = \"foo\"\nbinary_hash = \"sha256:old\"\n";
        let result = insert_binary_hash(content, "sha256:new").unwrap();
        assert!(result.contains("binary_hash = \"sha256:new\""));
        assert!(!result.contains("sha256:old"));
    }
}
