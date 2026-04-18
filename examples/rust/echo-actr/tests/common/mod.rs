#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use actr_hyper::{Hyper, HyperConfig, StaticTrust, WorkloadPackage};
use anyhow::{Context, Result};
use base64::Engine;

pub fn package_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PACKAGE_PATH") {
        return Some(PathBuf::from(path));
    }

    let default_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "dist/actrium-EchoService-{}-wasm32-unknown-unknown.actr",
        env!("CARGO_PKG_VERSION")
    ));
    default_path.exists().then_some(default_path)
}

pub fn pubkey_path() -> PathBuf {
    std::env::var("PUBKEY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public-key.json"))
}

pub fn verify_package(
    package_path: &Path,
    pubkey_path: &Path,
) -> Result<actr_pack::PackageManifest> {
    let package_bytes = std::fs::read(package_path)
        .with_context(|| format!("failed to read package: {}", package_path.display()))?;
    let verifying_key = load_verifying_key(pubkey_path)?;
    let verified =
        actr_pack::verify(&package_bytes, &verifying_key).context("package verification failed")?;
    Ok(verified.manifest)
}

pub async fn load_package(package_path: &Path, pubkey_path: &Path) -> Result<String> {
    let package_bytes = std::fs::read(package_path)
        .with_context(|| format!("failed to read package: {}", package_path.display()))?;
    let verifying_key = load_verifying_key(pubkey_path)?;
    let verified = actr_pack::verify(&package_bytes, &verifying_key)?;

    if verified.manifest.binary.target.starts_with("wasm32-") {
        let temp_dir = tempfile::TempDir::new().context("failed to create tempdir")?;
        let hyper = Hyper::new(
            HyperConfig::new(
                temp_dir.path(),
                Arc::new(StaticTrust::new(verifying_key.to_bytes()).context("invalid pubkey")?),
            ),
        )
        .await
        .context("failed to initialize Hyper")?;

        let loaded = hyper
            .load_workload_package(&WorkloadPackage::new(package_bytes.clone()))
            .await
            .context("failed to load package workload")?;
        return Ok(format!("{:?}", loaded.backend));
    }

    let binary = actr_pack::load_binary(&package_bytes)?;
    let suffix = match Path::new(&verified.manifest.binary.path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "dylib" => ".dylib",
        "so" => ".so",
        "dll" => ".dll",
        _ => ".bin",
    };

    let tempdir = tempfile::TempDir::new().context("failed to create tempdir")?;
    let temp_path = tempdir.path().join(format!("echo-guest{suffix}"));
    std::fs::write(&temp_path, binary)
        .with_context(|| format!("failed to write temp library: {}", temp_path.display()))?;

    unsafe {
        libloading::Library::new(&temp_path)
            .with_context(|| format!("failed to load library: {}", temp_path.display()))?;
    }

    Ok("Native".to_string())
}

fn load_verifying_key(path: &Path) -> Result<ed25519_dalek::VerifyingKey> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read pubkey file: {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text).context("invalid pubkey JSON")?;
    let public_b64 = value["public_key"]
        .as_str()
        .context("pubkey JSON missing public_key")?;
    let public_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_b64)
        .context("invalid public_key base64")?;
    let key_bytes: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("public_key must be exactly 32 bytes"))?;
    ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).context("invalid Ed25519 public key")
}
