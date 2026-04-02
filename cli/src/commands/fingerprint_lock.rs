#![allow(dead_code)]

//! Lock file generation for manifest.proto exports.
//!
//! Computes service fingerprints from exported proto files and writes
//! `manifest.lock.toml`.

use crate::error::Result;
use actr_config::ConfigParser;
use actr_service_compat::{Fingerprint, ProtoFile};
use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestLockFile {
    pub version: u32,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceLock>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceLock {
    pub name: String,
    pub fingerprint: String,
    pub files: Vec<FileLock>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileLock {
    pub name: String,
    pub fingerprint: String,
}

pub async fn write_manifest_lock(config_path: &Path) -> Result<PathBuf> {
    let config = ConfigParser::from_manifest_file(config_path)
        .with_context(|| format!("Failed to load manifest from {}", config_path.display()))?;

    let lock_path = config.config_dir.join("manifest.lock.toml");

    let service_lock = if config.exports.is_empty() {
        info!("No proto exports found; writing lock file without service fingerprint");
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

        let files = proto_files
            .iter()
            .map(|pf| {
                let file_fp = Fingerprint::calculate_proto_semantic_fingerprint(&pf.content)
                    .unwrap_or_else(|_| "error".to_string());
                debug!("{} -> {}", pf.name, file_fp);
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

    let lock_file = ManifestLockFile {
        version: 1,
        updated_at: Utc::now().to_rfc3339(),
        service: service_lock,
    };

    write_lock_file(&lock_path, &lock_file)?;
    Ok(lock_path)
}

fn write_lock_file(path: &PathBuf, lock: &ManifestLockFile) -> Result<()> {
    let content = toml::to_string_pretty(lock).context("Failed to serialize manifest.lock.toml")?;
    std::fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub fn read_lock_file(config_dir: &Path) -> Option<ManifestLockFile> {
    let path = config_dir.join("manifest.lock.toml");
    if !path.exists() {
        warn!("manifest.lock.toml not found at {}", path.display());
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}
