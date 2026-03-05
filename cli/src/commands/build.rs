//! Build command implementation
//!
//! Computes service fingerprint from exported proto files and writes Actr.lock.
//! This is a prerequisite for consumers to reference services by exact fingerprint.

use crate::error::Result;
use actr_config::ConfigParser;
use actr_version::{Fingerprint, ProtoFile};
use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Build command arguments
pub struct BuildArgs {
    /// Configuration file path
    pub config: String,
}

impl Default for BuildArgs {
    fn default() -> Self {
        Self {
            config: "Actr.toml".to_string(),
        }
    }
}

/// Lock file structure (Actr.lock)
#[derive(Debug, Serialize, Deserialize)]
pub struct ActrLockFile {
    /// Lock file format version
    pub version: u32,

    /// Timestamp of last update (RFC3339)
    pub updated_at: String,

    /// Service info (populated when this project exports protos)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceLock>,
}

/// Locked service info
#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceLock {
    /// Service name (from package.name)
    pub name: String,

    /// Semantic fingerprint of all exported proto files combined
    pub fingerprint: String,

    /// Per-file fingerprints
    pub files: Vec<FileLock>,
}

/// Per-proto-file lock entry
#[derive(Debug, Serialize, Deserialize)]
pub struct FileLock {
    /// File name (relative)
    pub name: String,

    /// Semantic fingerprint of this file
    pub fingerprint: String,
}

/// Execute the build command
pub async fn execute(args: BuildArgs) -> Result<()> {
    let config_path = Path::new(&args.config);
    info!("🔨 Building project from {}", args.config);

    let config = ConfigParser::from_file(config_path)
        .with_context(|| format!("Failed to load config from {}", args.config))?;

    let lock_path = config.config_dir.join("Actr.lock");

    let service_lock = if config.exports.is_empty() {
        info!("ℹ️  No proto exports — skipping fingerprint computation");
        None
    } else {
        let proto_files: Vec<ProtoFile> = config
            .exports
            .iter()
            .map(|pf| ProtoFile {
                name: pf.file_name().unwrap_or("unknown.proto").to_string(),
                content: pf.content.clone(),
                path: Some(pf.path.to_string_lossy().to_string()),
            })
            .collect();

        let fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
            .context("Failed to calculate service fingerprint")?;
        info!("📋 Service fingerprint: {fingerprint}");

        let files = proto_files
            .iter()
            .map(|pf| {
                let file_fp = Fingerprint::calculate_proto_semantic_fingerprint(&pf.content)
                    .unwrap_or_else(|_| "error".to_string());
                debug!("  {} → {}", pf.name, file_fp);
                FileLock {
                    name: pf.name.clone(),
                    fingerprint: file_fp,
                }
            })
            .collect();

        Some(ServiceLock {
            name: config.package.name.clone(),
            fingerprint,
            files,
        })
    };

    let lock_file = ActrLockFile {
        version: 1,
        updated_at: Utc::now().to_rfc3339(),
        service: service_lock,
    };

    write_lock_file(&lock_path, &lock_file)?;
    info!("✅ Actr.lock written to {}", lock_path.display());

    Ok(())
}

fn write_lock_file(path: &PathBuf, lock: &ActrLockFile) -> Result<()> {
    let content = toml::to_string_pretty(lock).context("Failed to serialize Actr.lock")?;
    std::fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Read an existing lock file (returns None if not found)
pub fn read_lock_file(config_dir: &Path) -> Option<ActrLockFile> {
    let path = config_dir.join("Actr.lock");
    if !path.exists() {
        warn!("Actr.lock not found at {}", path.display());
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}
