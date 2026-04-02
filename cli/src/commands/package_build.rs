use std::path::{Path, PathBuf};

use actr_config::ConfigParser;
use anyhow::{Context, Result};
use base64::Engine;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

pub struct PackageBuildInput {
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub key_path: PathBuf,
    pub output_path: PathBuf,
    pub target: String,
    pub resources: Vec<(String, PathBuf)>,
}

pub struct PackageBuildSummary {
    pub actr_type: String,
    pub target: String,
    pub binary_path: PathBuf,
    pub output_path: PathBuf,
    pub binary_hash: String,
    pub package_size: usize,
    pub public_key: String,
}

pub fn resolve_key_path(custom: Option<&Path>, config_keychain: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = custom {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = config_keychain {
        if let Some(stripped) = path.strip_prefix("~/") {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
            return Ok(home.join(stripped));
        }
        return Ok(PathBuf::from(path));
    }

    anyhow::bail!(
        "No signing key configured.\nSpecify --key, or set mfr.keychain in your CLI config."
    )
}

pub fn load_signing_key(key_path: &Path) -> Result<SigningKey> {
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

pub fn load_verifying_key(path: &Path) -> Result<ed25519_dalek::VerifyingKey> {
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

pub fn load_verifying_key_from_dev_key(path: &Path) -> Result<ed25519_dalek::VerifyingKey> {
    if !path.exists() {
        anyhow::bail!(
            "No key file found at {}. Specify --pubkey or run `actr pkg keygen` first.",
            path.display()
        );
    }

    load_verifying_key(path)
}

pub fn default_dist_output_path(config_path: &Path, target: &str) -> Result<PathBuf> {
    let file_name = default_package_file_name(config_path, target)?;
    let manifest_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    Ok(manifest_dir.join("dist").join(file_name))
}

pub fn default_pkg_output_path(config_path: &Path, target: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(default_package_file_name(
        config_path,
        target,
    )?))
}

fn default_package_file_name(config_path: &Path, target: &str) -> Result<String> {
    let config = ConfigParser::from_manifest_file(config_path).with_context(|| {
        format!(
            "Failed to parse manifest for default output path: {}",
            config_path.display()
        )
    })?;
    Ok(format!(
        "{}-{}-{}-{}.actr",
        config.package.actr_type.manufacturer,
        config.package.actr_type.name,
        config.package.actr_type.version,
        target
    ))
}

pub fn build_package(input: PackageBuildInput) -> Result<PackageBuildSummary> {
    let signing_key = load_signing_key(&input.key_path)?;
    let verifying_key = signing_key.verifying_key();

    let config = ConfigParser::from_manifest_file(&input.config_path).with_context(|| {
        format!(
            "Failed to parse manifest configuration: {}",
            input.config_path.display()
        )
    })?;

    let binary_bytes = std::fs::read(&input.binary_path)
        .with_context(|| format!("Failed to read binary: {}", input.binary_path.display()))?;

    let resources = input
        .resources
        .iter()
        .map(|(zip_path, local_path)| {
            let bytes = std::fs::read(local_path)
                .with_context(|| format!("Failed to read resource: {}", local_path.display()))?;
            Ok((zip_path.clone(), bytes))
        })
        .collect::<Result<Vec<_>>>()?;

    let proto_files = config
        .exports
        .iter()
        .map(|proto| {
            (
                proto.file_name().unwrap_or("unknown.proto").to_string(),
                proto.content.as_bytes().to_vec(),
            )
        })
        .collect::<Vec<_>>();

    let lock_file = {
        let lock_path = config.config_dir.join("manifest.lock.toml");
        if lock_path.exists() {
            Some(
                std::fs::read(&lock_path)
                    .with_context(|| format!("Failed to read {}", lock_path.display()))?,
            )
        } else {
            None
        }
    };

    let manifest = actr_pack::PackageManifest {
        manufacturer: config.package.actr_type.manufacturer.clone(),
        name: config.package.actr_type.name.clone(),
        version: config.package.actr_type.version.clone(),
        binary: actr_pack::BinaryEntry {
            path: "bin/actor.wasm".to_string(),
            target: input.target.clone(),
            hash: String::new(),
            size: None,
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: Some(actr_pack::compute_key_id(&verifying_key.to_bytes())),
        resources: vec![],
        proto_files: vec![],
        lock_file: None,
        metadata: actr_pack::ManifestMetadata {
            description: config.package.description.clone(),
            license: config.package.license.clone(),
        },
    };

    let package_bytes = actr_pack::pack(&actr_pack::PackOptions {
        manifest,
        binary_bytes: binary_bytes.clone(),
        resources,
        proto_files,
        signing_key,
        lock_file,
    })?;

    if let Some(parent) = input.output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
    }
    std::fs::write(&input.output_path, &package_bytes)
        .with_context(|| format!("Failed to write package: {}", input.output_path.display()))?;

    let binary_hash = hex::encode(Sha256::digest(&binary_bytes));
    let public_key = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());
    let actr_type = format!(
        "{}:{}:{}",
        config.package.actr_type.manufacturer,
        config.package.actr_type.name,
        config.package.actr_type.version
    );

    Ok(PackageBuildSummary {
        actr_type,
        target: input.target,
        binary_path: input.binary_path,
        output_path: input.output_path,
        binary_hash,
        package_size: package_bytes.len(),
        public_key,
    })
}

pub fn print_build_summary(summary: &PackageBuildSummary) {
    println!("Package built successfully");
    println!();
    println!("  type:        {}", summary.actr_type);
    println!("  target:      {}", summary.target);
    println!("  binary:      {}", summary.binary_path.display());
    println!("  binary_hash: {}...", &summary.binary_hash[..16]);
    println!("  output:      {}", summary.output_path.display());
    println!("  size:        {} bytes", summary.package_size);
    println!();
    println!("Public key (for verification):");
    println!("  {}", summary.public_key);
}
