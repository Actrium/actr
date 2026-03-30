//! `actr pkg` — Package management commands
//!
//! ## Subcommands
//!
//! ```text
//! actr pkg build    --binary FILE [--config manifest.toml] [--key FILE] [--output FILE]
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
use serde::Serialize;

#[derive(Args, Debug)]
pub struct PkgArgs {
    #[command(subcommand)]
    pub command: PkgCommand,
}

#[derive(Subcommand, Debug)]
pub enum PkgCommand {
    /// Build an .actr package from binary and config
    Build(PkgBuildArgs),
    /// Sign a manifest.toml package manifest with MFR private key (offline signing)
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

    /// manifest.toml config path
    #[arg(
        long,
        short = 'c',
        default_value = "manifest.toml",
        value_name = "FILE"
    )]
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

    /// Add a resource file to the package: --resource zip_path=local_path
    /// Can be specified multiple times.
    #[arg(long, value_parser = parse_resource_arg)]
    pub resource: Vec<(String, PathBuf)>,
}

#[derive(Args, Debug)]
pub struct PkgSignArgs {
    /// Path to manifest.toml config file
    #[arg(
        long,
        short = 'c',
        default_value = "manifest.toml",
        value_name = "FILE"
    )]
    pub config: PathBuf,

    /// Path to MFR signing key file (default: ~/.actr/dev-key.json)
    #[arg(long, short = 'k', value_name = "FILE")]
    pub key: Option<PathBuf>,

    /// Path to actor binary (for hash computation)
    #[arg(long, short = 'b', value_name = "FILE")]
    pub binary: Option<PathBuf>,

    /// Target platform (e.g., wasm32-wasip1, x86_64-unknown-linux-gnu)
    #[arg(long, short = 't', default_value = "wasm32-wasip1")]
    pub target: String,

    /// Output signature file (default: manifest.sig)
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

    /// Path to MFR keychain JSON file (used to verify publisher identity)
    #[arg(long, short = 'k', value_name = "FILE")]
    pub keychain: PathBuf,

    /// Actrix MFR endpoint URL (e.g., http://localhost:8081)
    #[arg(long, short = 'e', value_name = "URL")]
    pub endpoint: String,
}

#[derive(Serialize)]
struct SignablePublishBody<'a> {
    manufacturer: &'a str,
    name: &'a str,
    version: &'a str,
    target: &'a str,
    manifest: &'a str,
    signature: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    proto_files: Option<&'a serde_json::Value>,
    nonce: &'a str,
}

#[derive(Serialize)]
struct FinalPublishBody<'a> {
    manufacturer: &'a str,
    name: &'a str,
    version: &'a str,
    target: &'a str,
    manifest: &'a str,
    signature: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    proto_files: Option<&'a serde_json::Value>,
    nonce: &'a str,
    nonce_sig: &'a str,
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

/// Parse a `zip_path=local_path` resource argument.
fn parse_resource_arg(s: &str) -> Result<(String, PathBuf), String> {
    let (zip_path, local_path) = s
        .split_once('=')
        .ok_or_else(|| "resource must be in format 'zip_path=local_path'".to_string())?;
    Ok((zip_path.to_string(), PathBuf::from(local_path)))
}

// --- build (.actr package creation) ---

async fn execute_build(args: PkgBuildArgs) -> Result<()> {
    use sha2::{Digest, Sha256};

    // 1. Load signing key
    let key_path = resolve_key_path(args.key.as_deref())?;
    let signing_key = load_signing_key(&key_path)?;
    let verifying_key = signing_key.verifying_key();
    tracing::debug!(key_path = %key_path.display(), "signing key loaded");

    // 2. Read manifest.toml for package metadata
    let config_bytes = std::fs::read(&args.config)
        .with_context(|| format!("Failed to read config: {}", args.config.display()))?;
    let config_value: toml::Value =
        toml::from_slice(&config_bytes).with_context(|| "Invalid manifest.toml")?;
    let pkg = config_value
        .get("package")
        .ok_or_else(|| anyhow::anyhow!("manifest.toml missing [package] section"))?;

    // Support flat [package].{manufacturer,name,version}.
    let get_str = |key: &str| -> Result<String> {
        pkg.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("manifest.toml [package].{key} missing"))
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
        actr_type = %actr_type,
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
        signing_key_id: Some(actr_pack::compute_key_id(&verifying_key.to_bytes())),
        resources: vec![],
        proto_files: vec![], // will be populated below from exports
        lock_file: None,
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

    // 5. Read proto files from `exports` array
    //    Prefer [package].exports, fallback to top-level exports (backward compat)
    let config_dir = args
        .config
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut proto_files: Vec<(String, Vec<u8>)> = Vec::new();
    let exports = pkg
        .get("exports")
        .and_then(|e| e.as_array())
        .or_else(|| config_value.get("exports").and_then(|e| e.as_array()));
    if let Some(exports) = exports {
        for export_entry in exports {
            if let Some(proto_path_str) = export_entry.as_str() {
                let proto_path = config_dir.join(proto_path_str);
                match std::fs::read(&proto_path) {
                    Ok(content) => {
                        let filename = proto_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown.proto")
                            .to_string();
                        println!("  proto:       {}", filename);
                        proto_files.push((filename, content));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read proto file {:?}: {}", proto_path, e);
                    }
                }
            }
        }
    }

    // 5b. Read resource files (--resource zip_path=local_path)
    let resources: Vec<(String, Vec<u8>)> = args
        .resource
        .iter()
        .map(|(zip_path, local_path)| {
            let bytes = std::fs::read(local_path)
                .with_context(|| format!("Failed to read resource: {}", local_path.display()))?;
            println!("  resource:    {} ({} bytes)", zip_path, bytes.len());
            Ok((zip_path.clone(), bytes))
        })
        .collect::<Result<_, anyhow::Error>>()?;

    // 6. Load manifest.lock.toml if it exists (packed into the .actr for dependency auditing)
    let lock_file = {
        let lock_path = config_dir.join("manifest.lock.toml");
        if lock_path.exists() {
            let bytes = std::fs::read(&lock_path)
                .with_context(|| format!("Failed to read {}", lock_path.display()))?;
            println!("  lock:        manifest.lock.toml ({} bytes)", bytes.len());
            Some(bytes)
        } else {
            None
        }
    };

    // 7. Pack
    let opts = actr_pack::PackOptions {
        manifest,
        binary_bytes: binary_bytes.clone(),
        resources,
        proto_files,
        signing_key: signing_key.clone(),
        lock_file,
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

// --- sign (offline signing of manifest.toml → manifest.toml + manifest.sig) ---
//
// Parses manifest.toml (same config format as `build`), reads binary + proto files,
// builds a PackageManifest, serializes via to_toml(), and signs.
// Output:
//   1. Manifest TOML file (manifest.toml) — the exact bytes that were signed
//   2. A .sig file (manifest.sig) — 64 bytes raw Ed25519 signature
//
// The signed content is byte-level identical to what `actr pkg build` produces.

async fn execute_sign(args: PkgSignArgs) -> Result<()> {
    use ed25519_dalek::Signer;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    // 1. Load signing key
    let key_path = resolve_key_path(args.key.as_deref())?;
    let signing_key = load_signing_key(&key_path)?;
    let verifying_key = signing_key.verifying_key();
    let key_id = actr_pack::compute_key_id(&verifying_key.to_bytes());

    // 2. Read manifest.toml as config (same parsing as build)
    let config_path = &args.config;
    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "config file not found: {}",
            config_path.display()
        ));
    }
    let config_bytes = std::fs::read(config_path)?;
    let config_value: toml::Value =
        toml::from_slice(&config_bytes).with_context(|| "Invalid manifest.toml")?;
    let pkg = config_value
        .get("package")
        .ok_or_else(|| anyhow::anyhow!("manifest.toml missing [package] section"))?;

    let get_str = |key: &str| -> Result<String> {
        pkg.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("manifest.toml [package].{key} missing"))
    };

    let manufacturer = get_str("manufacturer")?;
    let name = get_str("name")?;
    let version = get_str("version")?;

    // 3. Compute binary hash if binary provided
    let (binary_hash, binary_size) = if let Some(binary_path) = &args.binary {
        let binary_data = std::fs::read(binary_path)
            .with_context(|| format!("Failed to read binary: {}", binary_path.display()))?;
        let hash = Sha256::digest(&binary_data);
        println!(
            "  binary:    {} ({} bytes)",
            binary_path.display(),
            binary_data.len()
        );
        (hex::encode(hash), Some(binary_data.len() as u64))
    } else {
        (String::new(), None)
    };

    // 4. Read proto files from `exports` array (same as build)
    //    Prefer [package].exports, fallback to top-level exports (backward compat)
    let config_dir = args
        .config
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut proto_entries = vec![];
    let exports = pkg
        .get("exports")
        .and_then(|e| e.as_array())
        .or_else(|| config_value.get("exports").and_then(|e| e.as_array()));
    if let Some(exports) = exports {
        for export_entry in exports {
            if let Some(proto_path_str) = export_entry.as_str() {
                let proto_path = config_dir.join(proto_path_str);
                match std::fs::read(&proto_path) {
                    Ok(content) => {
                        let filename = proto_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown.proto")
                            .to_string();
                        let hash = hex::encode(Sha256::digest(&content));
                        println!("  proto:     {} (hash: {}...)", filename, &hash[..16]);
                        proto_entries.push(actr_pack::ProtoFileEntry {
                            name: filename.clone(),
                            path: format!("proto/{}", filename),
                            hash,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read proto file {:?}: {}", proto_path, e);
                    }
                }
            }
        }
    }

    // 5. Build PackageManifest (same structure as build)
    let manifest = actr_pack::PackageManifest {
        manufacturer: manufacturer.clone(),
        name: name.clone(),
        version: version.clone(),
        binary: actr_pack::BinaryEntry {
            path: "bin/actor.wasm".to_string(),
            target: args.target.clone(),
            hash: binary_hash,
            size: binary_size,
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: Some(key_id.clone()),
        resources: vec![],
        proto_files: proto_entries,
        lock_file: None,
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

    // 6. Serialize via to_toml() (byte-level consistent with build)
    let manifest_toml = manifest
        .to_toml()
        .map_err(|e| anyhow::anyhow!("Failed to serialize manifest: {e}"))?;
    let manifest_bytes = manifest_toml.as_bytes();

    // 7. Sign (raw 64-byte Ed25519, same as actr_pack::pack)
    let signature = signing_key.sign(manifest_bytes);
    let sig_bytes = signature.to_bytes();

    // 8. Write manifest TOML (the exact bytes that were signed)
    let manifest_path = {
        let mut p = args.config.clone();
        p.set_file_name("manifest.toml");
        p
    };
    std::fs::write(&manifest_path, manifest_bytes)
        .with_context(|| format!("Failed to write manifest: {}", manifest_path.display()))?;

    // 9. Write manifest.sig (64 bytes raw Ed25519 signature)
    let sig_path = args.output.unwrap_or_else(|| {
        let mut p = args.config.clone();
        p.set_file_name("manifest.sig");
        p
    });
    {
        let mut f = std::fs::File::create(&sig_path)?;
        f.write_all(&sig_bytes)?;
    }

    println!("✅ Manifest signed successfully");
    println!("  manifest:  {} (signed content)", manifest_path.display());
    println!("  sig file:  {} (64 bytes raw Ed25519)", sig_path.display());
    println!("  key_id:    {}", key_id);
    println!("  actr_type: {}:{}:{}", manufacturer, name, version);
    println!("  target:    {}", args.target);

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

    // 4. Check signing_key_id consistency with the provided public key
    if let Some(ref manifest_key_id) = verified.manifest.signing_key_id {
        let expected_key_id = actr_pack::compute_key_id(&pubkey.to_bytes());
        if manifest_key_id != &expected_key_id {
            anyhow::bail!(
                "signing_key_id mismatch: manifest says '{}' but the provided public key fingerprint is '{}'. \
                 This package will fail verification in Production mode. \
                 Rebuild with 'actr pkg build' using the correct signing key.",
                manifest_key_id,
                expected_key_id,
            );
        }
    } else {
        anyhow::bail!(
            "Package manifest has no 'signing_key_id'. \
             This package will be rejected in Production mode. \
             Rebuild with the latest 'actr pkg build' to embed a signing_key_id."
        );
    }

    println!("Package verification passed");
    println!();
    println!("  manufacturer: {}", verified.manifest.manufacturer);
    println!("  type:         {}", verified.manifest.actr_type_str());
    println!("  binary:       {}", verified.manifest.binary.path);
    println!(
        "  binary_hash:  {}...",
        &verified.manifest.binary.hash[..16]
    );
    println!("  target:       {}", verified.manifest.binary.target);
    if let Some(ref key_id) = verified.manifest.signing_key_id {
        println!("  signing_key:  {}", key_id);
    }
    if !verified.manifest.resources.is_empty() {
        println!("  resources:    {}", verified.manifest.resources.len());
    }

    Ok(())
}

// --- publish (register package to MFR registry) ---
//
// Extracts the manifest (manifest.toml) and signature (manifest.sig) that were
// already created during `actr pkg build`, then forwards them to the
// MFR registry as-is.  No re-signing is performed.
//
// This guarantees that the signature MFR validates is the exact same
// signature the Hyper runtime will verify at load time.

async fn execute_publish(args: PkgPublishArgs) -> Result<()> {
    // 1. Read .actr package
    tracing::debug!("reading .actr package: {:?}", args.package);
    let package_bytes = std::fs::read(&args.package)
        .with_context(|| format!("Failed to read package: {}", args.package.display()))?;

    // 2. Extract manifest TOML and signature from the .actr ZIP
    let manifest_str = actr_pack::read_manifest_raw(&package_bytes)
        .with_context(|| "Failed to read manifest from .actr package")?;
    let manifest = actr_pack::PackageManifest::from_toml(&manifest_str)
        .with_context(|| "Failed to parse manifest TOML")?;
    let sig_raw = actr_pack::read_signature(&package_bytes)
        .with_context(|| "Failed to read manifest.sig from .actr package")?;

    // 3. Identity verification: keychain private key proves publisher is MFR owner.
    //    We only check that the package's signing_key_id matches the keychain,
    //    ensuring the package was built with the same key.
    //    The MFR server performs the real signature verification.
    tracing::debug!(
        "loading keychain for identity verification: {:?}",
        args.keychain
    );
    let keychain_content = std::fs::read_to_string(&args.keychain)
        .with_context(|| format!("Failed to read keychain: {}", args.keychain.display()))?;
    let keychain: serde_json::Value =
        serde_json::from_str(&keychain_content).with_context(|| "Invalid keychain JSON")?;

    // Derive signing key and key_id from keychain private key
    let kc_privkey_b64 = keychain["private_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Keychain missing 'private_key' field"))?;
    let kc_privkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(kc_privkey_b64)
        .with_context(|| "Invalid private key in keychain")?;
    let kc_arr: [u8; 32] = kc_privkey_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("Private key must be 32 bytes"))?;
    let kc_signing_key = ed25519_dalek::SigningKey::from_bytes(&kc_arr);
    let kc_key_id = actr_pack::compute_key_id(&kc_signing_key.verifying_key().to_bytes());

    // Check signing_key_id consistency: package must be built with the same key
    match manifest.signing_key_id {
        Some(ref manifest_key_id) if manifest_key_id == &kc_key_id => {
            // OK — build key matches publish key
        }
        Some(ref manifest_key_id) => {
            anyhow::bail!(
                "Key mismatch: package was built with '{}' but keychain key is '{}'. \
                 Rebuild with the correct MFR key, or use the matching keychain.",
                manifest_key_id,
                kc_key_id
            );
        }
        None => {
            anyhow::bail!(
                "Package manifest has no 'signing_key_id'. \
                 Rebuild with the latest 'actr pkg build' to embed a signing_key_id."
            );
        }
    }

    // Encode raw 64-byte signature as base64 for the MFR API
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&sig_raw);

    println!("📦 Publishing package: {}", manifest.actr_type_str());
    println!("   manufacturer: {}", manifest.manufacturer);
    println!("   name:         {}", manifest.name);
    println!("   version:      {}", manifest.version);
    println!("   target:       {}", manifest.binary.target);
    println!("   signing_key:  {}", kc_key_id);
    println!("✅ Identity verified (keychain matches package)");
    println!("🔐 Forwarding original signature (no re-signing)");

    // 3. Extract proto files from .actr package for MFR filing
    let proto_files = actr_pack::read_proto_files(&package_bytes).unwrap_or_default();
    let proto_filing = if !proto_files.is_empty() {
        let proto_entries: Vec<serde_json::Value> = proto_files
            .iter()
            .map(|(name, content)| {
                serde_json::json!({
                    "name": name,
                    "content": String::from_utf8_lossy(content),
                })
            })
            .collect();
        println!("📋 Proto files for filing: {} file(s)", proto_entries.len());
        Some(serde_json::json!({
            "protobufs": proto_entries,
        }))
    } else {
        None
    };

    // 4. Resolve endpoint (strip trailing /ais if present)
    let endpoint = args
        .endpoint
        .trim_end_matches("/ais")
        .trim_end_matches('/')
        .to_string();

    // 5. Request Challenge-Response nonce from MFR server
    let base_url = endpoint.trim_end_matches('/');
    let nonce_url = format!("{}/mfr/pkg/nonce", base_url);
    let client = reqwest::Client::new();

    println!("🔑 Requesting publish nonce...");
    let nonce_resp = client
        .post(&nonce_url)
        .json(&serde_json::json!({ "manufacturer": manifest.manufacturer }))
        .send()
        .await
        .with_context(|| format!("Failed to request nonce from: {}", nonce_url))?;

    if !nonce_resp.status().is_success() {
        let status = nonce_resp.status();
        let body = nonce_resp.text().await.unwrap_or_default();
        anyhow::bail!("Nonce request failed (HTTP {}): {}", status, body);
    }

    let nonce_json: serde_json::Value = nonce_resp
        .json()
        .await
        .with_context(|| "Failed to parse nonce response")?;
    let nonce_b64 = nonce_json["nonce"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Nonce response missing 'nonce' field"))?
        .to_string();

    // Build the signable publish body (everything except nonce_sig) and authorize it
    // with the one-time nonce.
    let nonce_bytes = base64::engine::general_purpose::STANDARD
        .decode(&nonce_b64)
        .with_context(|| "Invalid nonce base64 from server")?;

    let signable_body = SignablePublishBody {
        manufacturer: &manifest.manufacturer,
        name: &manifest.name,
        version: &manifest.version,
        target: &manifest.binary.target,
        manifest: &manifest_str,
        signature: &sig_b64,
        proto_files: proto_filing.as_ref(),
        nonce: &nonce_b64,
    };
    let signable_body_bytes = serde_json::to_vec(&signable_body)
        .with_context(|| "Failed to serialize signable publish body")?;

    let nonce_sig_b64 = {
        use ed25519_dalek::Signer;
        use sha2::{Digest, Sha256};

        let body_hash = hex::encode(Sha256::digest(&signable_body_bytes));
        let nonce_hex = hex::encode(&nonce_bytes);
        let payload = format!(
            "ACTR-PUBLISH-V1\nmanufacturer={}\nmethod=POST\npath=/mfr/pkg/publish\nnonce={}\nbody_sha256={}",
            manifest.manufacturer, nonce_hex, body_hash
        );
        let sig = kc_signing_key.sign(payload.as_bytes());
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    };
    println!("✅ Nonce signed (challenge-response)");

    // 6. POST /mfr/pkg/publish
    let publish_url = format!("{}/mfr/pkg/publish", base_url);
    println!("📡 Publishing to: {}", publish_url);

    let publish_body_bytes = serde_json::to_vec(&FinalPublishBody {
        manufacturer: &manifest.manufacturer,
        name: &manifest.name,
        version: &manifest.version,
        target: &manifest.binary.target,
        manifest: &manifest_str,
        signature: &sig_b64,
        proto_files: proto_filing.as_ref(),
        nonce: &nonce_b64,
        nonce_sig: &nonce_sig_b64,
    })
    .with_context(|| "Failed to serialize publish request body")?;

    let resp = client
        .post(&publish_url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(publish_body_bytes)
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
