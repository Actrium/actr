//! `actr pkg` — Package management commands
//!
//! Provides package lifecycle operations including signing for MFR-based publishing.
//!
//! ## Subcommands
//!
//! ```text
//! actr pkg sign --keychain=FILE [--binary=FILE] [--config=actr.toml] [--output=FILE]
//! ```

use clap::{Args, Subcommand};
use std::path::PathBuf;
use tracing::{debug, info, warn};

#[derive(Args, Debug)]
pub struct PkgArgs {
    #[command(subcommand)]
    pub command: PkgCommand,
}

#[derive(Subcommand, Debug)]
pub enum PkgCommand {
    /// Sign actr.toml with MFR private key for package publishing
    Sign(PkgSignArgs),
}

#[derive(Args, Debug)]
pub struct PkgSignArgs {
    /// Path to MFR keychain JSON file (from actrix MFR registration)
    #[arg(long, short = 'k', value_name = "FILE")]
    pub keychain: PathBuf,

    /// Path to actor binary to compute sha256 hash (optional)
    #[arg(long, short = 'b', value_name = "FILE")]
    pub binary: Option<PathBuf>,

    /// Path to actr.toml
    #[arg(long, short = 'c', default_value = "actr.toml", value_name = "FILE")]
    pub config: PathBuf,

    /// Output signature file (defaults to <config>.sig)
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
}

pub async fn execute(args: PkgArgs) -> anyhow::Result<()> {
    match args.command {
        PkgCommand::Sign(sign_args) => execute_sign(sign_args).await,
    }
}

async fn execute_sign(args: PkgSignArgs) -> anyhow::Result<()> {
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    use std::io::Write;

    // 1. Read keychain JSON
    debug!("reading keychain file: {:?}", args.keychain);
    let keychain_content = std::fs::read_to_string(&args.keychain)
        .map_err(|e| anyhow::anyhow!("failed to read keychain file {:?}: {}", args.keychain, e))?;
    let keychain: serde_json::Value = serde_json::from_str(&keychain_content)
        .map_err(|e| anyhow::anyhow!("invalid keychain JSON: {}", e))?;
    let private_key_b64 = keychain["private_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("keychain missing 'private_key' field"))?;

    // 2. Decode private key (Ed25519, 32 bytes)
    let private_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(private_key_b64)
        .map_err(|e| anyhow::anyhow!("invalid private key base64 encoding: {}", e))?;
    let key_array: [u8; 32] = private_key_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private key must be exactly 32 bytes (Ed25519)"))?;
    let signing_key = SigningKey::from_bytes(&key_array);
    debug!("signing key loaded");

    // 3. Read actr.toml
    let config_path = &args.config;
    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "config file not found: {}",
            config_path.display()
        ));
    }
    let config_content = std::fs::read_to_string(config_path)
        .map_err(|e| anyhow::anyhow!("failed to read {:?}: {}", config_path, e))?;
    debug!("loaded config: {} bytes", config_content.len());

    // 4. If binary is specified, compute sha256 and write to actr.toml
    let final_config = if let Some(binary_path) = &args.binary {
        info!("computing sha256 for binary: {}", binary_path.display());
        let binary_data = std::fs::read(binary_path)
            .map_err(|e| anyhow::anyhow!("failed to read binary {:?}: {}", binary_path, e))?;
        let mut hasher = Sha256::new();
        hasher.update(&binary_data);
        let hash_hex = hex::encode(hasher.finalize());
        let binary_hash = format!("sha256:{}", hash_hex);

        let updated = insert_binary_hash(&config_content, &binary_hash)?;

        std::fs::write(config_path, &updated)
            .map_err(|e| anyhow::anyhow!("failed to write back actr.toml: {}", e))?;
        info!("binary_hash written to {}", config_path.display());
        println!("binary_hash = \"{}\"", binary_hash);
        updated
    } else {
        warn!("no binary specified; binary_hash field will not be updated");
        config_content
    };

    // 5. Ed25519 sign the actr.toml content
    let manifest_bytes = final_config.as_bytes();
    let signature = signing_key.sign(manifest_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
    debug!("signature computed ({} bytes)", signature.to_bytes().len());

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
        let mut f = std::fs::File::create(&sig_path).map_err(|e| {
            anyhow::anyhow!("failed to create signature file {:?}: {}", sig_path, e)
        })?;
        writeln!(f, "{}", sig_b64)?;
    }
    info!("signature written to {}", sig_path.display());

    // 7. Extract type_str for display
    let type_str = extract_type_str(&final_config);

    // 8. Compute manifest sha256 for display
    let mut manifest_hasher = Sha256::new();
    manifest_hasher.update(manifest_bytes);
    let manifest_hash = hex::encode(manifest_hasher.finalize());

    println!("Package signed successfully");
    println!(
        "  type:            {}",
        type_str.as_deref().unwrap_or("(unknown)")
    );
    println!("  manifest sha256: {}...", &manifest_hash[..16]);
    println!("  signature:       {}...", &sig_b64[..16]);
    println!("  sig file:        {}", sig_path.display());

    Ok(())
}

/// Insert or update `binary_hash` in the `[package]` section of actr.toml content.
///
/// If `binary_hash` key already exists anywhere, replace that line in-place.
/// Otherwise, insert after the `[package]` section header.
fn insert_binary_hash(content: &str, binary_hash: &str) -> anyhow::Result<String> {
    let hash_line = format!("binary_hash = \"{}\"", binary_hash);

    // binary_hash already exists: replace line by line
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
        // Preserve trailing newline (if the original file had one)
        let trailing_newline = if content.ends_with('\n') { "\n" } else { "" };
        return Ok(format!("{}{}", replaced, trailing_newline));
    }

    // Not present: append at the end of [package] section (before the next section or EOF)
    let mut result = String::with_capacity(content.len() + hash_line.len() + 2);
    let mut in_package = false;
    let mut inserted = false;
    for line in content.lines() {
        if !inserted {
            let trimmed = line.trim();
            if trimmed == "[package]" {
                in_package = true;
            } else if in_package && (trimmed.starts_with('[') && trimmed.ends_with(']')) {
                // About to enter the next section; insert before it
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
        // Insert at end of file
        result.push_str(&hash_line);
        result.push('\n');
    }
    Ok(result)
}

/// Extract `manufacturer:name:version` (or `manufacturer:name`) from actr.toml content.
///
/// Uses a lightweight line-by-line scan; does not depend on actr-config.
fn extract_type_str(content: &str) -> Option<String> {
    let mut manufacturer = None;
    let mut name = None;
    let mut version = None;

    for line in content.lines() {
        let line = line.trim();
        if let Some(val) = extract_toml_str_value(line, "manufacturer") {
            manufacturer = Some(val);
        } else if let Some(val) = extract_toml_str_value(line, "name") {
            if name.is_none() {
                name = Some(val);
            }
        } else if let Some(val) = extract_toml_str_value(line, "version") {
            if version.is_none() {
                version = Some(val);
            }
        }
    }

    match (manufacturer, name, version) {
        (Some(m), Some(n), Some(v)) => Some(format!("{}:{}:{}", m, n, v)),
        (Some(m), Some(n), None) => Some(format!("{}:{}", m, n)),
        _ => None,
    }
}

fn extract_toml_str_value(line: &str, key: &str) -> Option<String> {
    // Match `key = "value"` or `key="value"`
    let prefix = format!("{} =", key);
    let prefix_nospace = format!("{}=", key);
    let rest = if line.starts_with(&prefix) {
        &line[prefix.len()..]
    } else if line.starts_with(&prefix_nospace) {
        &line[prefix_nospace.len()..]
    } else {
        return None;
    };
    let rest = rest.trim();
    if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        Some(rest[1..rest.len() - 1].to_string())
    } else {
        None
    }
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

    #[test]
    fn test_extract_type_str_full() {
        let content = "[package]\nmanufacturer = \"acme\"\nname = \"bot\"\nversion = \"2.0.0\"\n";
        assert_eq!(
            extract_type_str(content),
            Some("acme:bot:2.0.0".to_string())
        );
    }

    #[test]
    fn test_extract_type_str_no_version() {
        let content = "[package]\nmanufacturer = \"acme\"\nname = \"bot\"\n";
        assert_eq!(extract_type_str(content), Some("acme:bot".to_string()));
    }
}
