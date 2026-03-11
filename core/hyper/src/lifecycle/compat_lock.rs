//! compat.lock.toml - Runtime compatibility negotiation cache
//!
//! This file is created when service discovery cannot find an exact match
//! but finds a compatible match. Its existence indicates the system is in
//! SUB-HEALTHY state.
//!
//! ## Features
//! - Cache negotiation results to avoid repeated compatibility checks
//! - Record system health state for operations monitoring
//! - Provide a fast startup path by trying known compatible versions first
//!
//! ## Storage Location
//! This file is stored in the OS temporary directory, not the project directory:
//! - Linux/macOS: `/tmp/actr/<project_hash>/compat.lock.toml`
//! - Windows: `%TEMP%\actr\<project_hash>\compat.lock.toml`
//!
//! `project_hash` is a unique hash computed from the project root absolute path,
//! ensuring each Actor instance on the same machine has its own independent cache.
//!
//! ## Note
//! This file should not be committed to version control as it reflects runtime state.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info, warn};

/// Filename constant
const COMPAT_LOCK_FILENAME: &str = "compat.lock.toml";

/// Subdirectory name under the temp directory
const ACTR_TEMP_DIR: &str = "actr";

/// Default cache expiration time (24 hours)
const DEFAULT_TTL_HOURS: i64 = 24;

/// Compute a unique hash from the project root directory path
///
/// Returns a short hash string (16 characters) used to create temp directory subpaths
fn compute_project_hash(project_root: &Path) -> String {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let path_str = canonical.to_string_lossy();
    let mut hasher = Sha256::new();
    hasher.update(path_str.as_bytes());
    let result = hasher.finalize();
    // Take the first 8 bytes (16 hex characters) as the hash
    hex::encode(&result[..8])
}

/// Get the storage directory for compat.lock.toml
///
/// Path format: `<temp_dir>/actr/<project_hash>/`
fn get_compat_lock_dir(project_root: &Path) -> PathBuf {
    let temp_dir = std::env::temp_dir();
    let project_hash = compute_project_hash(project_root);
    temp_dir.join(ACTR_TEMP_DIR).join(project_hash)
}

/// Compatibility negotiation entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationEntry {
    /// Service name (e.g. "user-service")
    pub service_name: String,

    /// Requested fingerprint (version expected by the client)
    pub requested_fingerprint: String,

    /// Actually resolved fingerprint (version provided by the server)
    pub resolved_fingerprint: String,

    /// Compatibility check result
    pub compatibility_check: CompatibilityCheck,

    /// Negotiation time
    pub negotiated_at: DateTime<Utc>,

    /// Expiration time
    pub expires_at: DateTime<Utc>,
}

/// Compatibility check result
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityCheck {
    /// Fully compatible (exact match)
    ExactMatch,
    /// Backward compatible
    BackwardCompatible,
    /// Breaking changes (should not appear in lock file)
    BreakingChanges,
}

impl std::fmt::Display for CompatibilityCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompatibilityCheck::ExactMatch => write!(f, "exact_match"),
            CompatibilityCheck::BackwardCompatible => write!(f, "backward_compatible"),
            CompatibilityCheck::BreakingChanges => write!(f, "breaking_changes"),
        }
    }
}

/// compat.lock.toml file structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompatLockFile {
    /// File header comment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _comment: Option<String>,

    /// Negotiation entry list
    #[serde(default)]
    pub negotiation: Vec<NegotiationEntry>,
}

impl CompatLockFile {
    /// Create a new empty lock file
    pub fn new() -> Self {
        Self {
            _comment: Some(
                "This file indicates the system is in SUB-HEALTHY state.\n\
                 Consider running 'actr install --force-update' to update dependencies."
                    .to_string(),
            ),
            negotiation: Vec::new(),
        }
    }

    /// Load from file
    pub async fn load(base_path: &Path) -> Result<Option<Self>, CompatLockError> {
        let file_path = base_path.join(COMPAT_LOCK_FILENAME);

        if !file_path.exists() {
            return Ok(None);
        }

        let content =
            fs::read_to_string(&file_path)
                .await
                .map_err(|e| CompatLockError::IoError {
                    path: file_path.clone(),
                    source: e,
                })?;

        let lock_file: Self =
            toml::from_str(&content).map_err(|e| CompatLockError::ParseError {
                path: file_path,
                source: e,
            })?;

        Ok(Some(lock_file))
    }

    /// Save to file
    pub async fn save(&self, base_path: &Path) -> Result<(), CompatLockError> {
        // Ensure directory exists (temp directory may not exist)
        if !base_path.exists() {
            fs::create_dir_all(base_path)
                .await
                .map_err(|e| CompatLockError::IoError {
                    path: base_path.to_path_buf(),
                    source: e,
                })?;
            debug!(
                "Created compat.lock cache directory: {}",
                base_path.display()
            );
        }

        let file_path = base_path.join(COMPAT_LOCK_FILENAME);

        let content = toml::to_string_pretty(self)
            .map_err(|e| CompatLockError::SerializeError { source: e })?;

        // Add file header comment
        let full_content = format!(
            "# compat.lock.toml - Compatibility negotiation cache\n\
             # This file indicates the system is in SUB-HEALTHY state.\n\
             # Consider running 'actr install --force-update' to update dependencies.\n\
             # Location: {}\n\n\
             {content}",
            file_path.display()
        );

        fs::write(&file_path, full_content)
            .await
            .map_err(|e| CompatLockError::IoError {
                path: file_path,
                source: e,
            })?;

        Ok(())
    }

    /// Remove lock file (called when system recovers to healthy state)
    pub async fn remove(base_path: &Path) -> Result<bool, CompatLockError> {
        let file_path = base_path.join(COMPAT_LOCK_FILENAME);

        if file_path.exists() {
            fs::remove_file(&file_path)
                .await
                .map_err(|e| CompatLockError::IoError {
                    path: file_path,
                    source: e,
                })?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Find negotiation entry for a service
    pub fn find_entry(&self, service_name: &str) -> Option<&NegotiationEntry> {
        self.negotiation
            .iter()
            .find(|e| e.service_name == service_name)
    }

    /// Find a non-expired negotiation entry
    pub fn find_valid_entry(&self, service_name: &str) -> Option<&NegotiationEntry> {
        let now = Utc::now();
        self.negotiation
            .iter()
            .find(|e| e.service_name == service_name && e.expires_at > now)
    }

    /// Add or update a negotiation entry
    pub fn upsert_entry(&mut self, entry: NegotiationEntry) {
        // Remove existing entry with the same name
        self.negotiation
            .retain(|e| e.service_name != entry.service_name);
        // Add the new entry
        self.negotiation.push(entry);
    }

    /// Clean up expired entries
    pub fn cleanup_expired(&mut self) -> usize {
        let now = Utc::now();
        let before = self.negotiation.len();
        self.negotiation.retain(|e| e.expires_at > now);
        before - self.negotiation.len()
    }

    /// Check whether the file exists (i.e. whether the system is in sub-healthy state)
    pub async fn exists(base_path: &Path) -> bool {
        base_path.join(COMPAT_LOCK_FILENAME).exists()
    }

    /// Check whether there are any valid non-exact-match entries (sub-healthy state)
    pub fn is_sub_healthy(&self) -> bool {
        let now = Utc::now();
        self.negotiation.iter().any(|e| {
            e.expires_at > now && e.compatibility_check == CompatibilityCheck::BackwardCompatible
        })
    }
}

impl NegotiationEntry {
    /// Create a new negotiation entry
    pub fn new(
        service_name: String,
        requested_fingerprint: String,
        resolved_fingerprint: String,
        compatibility_check: CompatibilityCheck,
    ) -> Self {
        let now = Utc::now();
        Self {
            service_name,
            requested_fingerprint,
            resolved_fingerprint,
            compatibility_check,
            negotiated_at: now,
            expires_at: now + Duration::hours(DEFAULT_TTL_HOURS),
        }
    }

    /// Check whether this entry has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// compat.lock related errors
#[derive(Debug, thiserror::Error)]
pub enum CompatLockError {
    #[error("IO error at {path}: {source}")]
    IoError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Parse error at {path}: {source}")]
    ParseError {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Serialize error: {source}")]
    SerializeError {
        #[source]
        source: toml::ser::Error,
    },
}

/// Compatibility negotiation manager - used at runtime
pub struct CompatLockManager {
    /// Lock file directory (computed temp directory path)
    base_path: PathBuf,
    /// Project root directory (used for logging)
    #[allow(dead_code)]
    project_root: PathBuf,
    /// Cached lock file contents
    cached: Option<CompatLockFile>,
}

impl CompatLockManager {
    /// Create a new manager
    ///
    /// # Arguments
    /// * `project_root` - Path to the project root directory, used to compute a unique cache directory
    ///
    /// # Storage Location
    /// Files will be stored at `<temp_dir>/actr/<project_hash>/compat.lock.toml`
    pub fn new(project_root: PathBuf) -> Self {
        let base_path = get_compat_lock_dir(&project_root);
        debug!(
            "CompatLockManager initialized: project_root={}, cache_dir={}",
            project_root.display(),
            base_path.display()
        );
        Self {
            base_path,
            project_root,
            cached: None,
        }
    }

    /// Get the storage directory of the compat.lock file
    pub fn cache_dir(&self) -> &Path {
        &self.base_path
    }

    /// Load the lock file
    pub async fn load(&mut self) -> Result<Option<&CompatLockFile>, CompatLockError> {
        self.cached = CompatLockFile::load(&self.base_path).await?;
        Ok(self.cached.as_ref())
    }

    /// Get the cached lock file
    pub fn get_cached(&self) -> Option<&CompatLockFile> {
        self.cached.as_ref()
    }

    /// Record a negotiation result
    ///
    /// Called when a service is discovered:
    /// - If exact match, try to remove the corresponding negotiation entry
    /// - If compatible match, add/update the negotiation entry
    pub async fn record_negotiation(
        &mut self,
        service_name: &str,
        requested_fingerprint: &str,
        resolved_fingerprint: &str,
        is_exact_match: bool,
        compatibility_check: CompatibilityCheck,
    ) -> Result<(), CompatLockError> {
        if is_exact_match {
            // Exact match: try to remove the old negotiation entry
            if let Some(ref mut lock_file) = self.cached {
                lock_file
                    .negotiation
                    .retain(|e| e.service_name != service_name);

                // If all entries have been cleared, remove the file
                if lock_file.negotiation.is_empty() {
                    CompatLockFile::remove(&self.base_path).await?;
                    self.cached = None;
                    info!(
                        "SYSTEM HEALTHY: all dependencies are exact matches, removed compat.lock.toml"
                    );
                } else {
                    lock_file.save(&self.base_path).await?;
                }
            }
        } else {
            // Compatible match: record to lock file
            let entry = NegotiationEntry::new(
                service_name.to_string(),
                requested_fingerprint.to_string(),
                resolved_fingerprint.to_string(),
                compatibility_check,
            );

            let lock_file = self.cached.get_or_insert_with(CompatLockFile::new);
            lock_file.upsert_entry(entry);
            lock_file.save(&self.base_path).await?;

            warn!(
                "🟡 SYSTEM SUB-HEALTHY: Service '{}' using compatible fingerprint ({}) instead of exact match ({}). \
                 Run 'actr install --force-update' to restore health.",
                service_name,
                &resolved_fingerprint[..20.min(resolved_fingerprint.len())],
                &requested_fingerprint[..20.min(requested_fingerprint.len())],
            );
        }

        Ok(())
    }

    /// Find a cached compatible version (for fast startup)
    pub fn find_cached_compatible(
        &self,
        service_name: &str,
        requested_fingerprint: &str,
    ) -> Option<&NegotiationEntry> {
        self.cached.as_ref().and_then(|lock_file| {
            lock_file
                .find_valid_entry(service_name)
                .filter(|entry| entry.requested_fingerprint == requested_fingerprint)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_compat_lock_file_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // Create and save
        let mut lock_file = CompatLockFile::new();
        lock_file.upsert_entry(NegotiationEntry::new(
            "user-service".to_string(),
            "sha256:old".to_string(),
            "sha256:new".to_string(),
            CompatibilityCheck::BackwardCompatible,
        ));
        lock_file.save(base_path).await.unwrap();

        // Verify file exists
        assert!(CompatLockFile::exists(base_path).await);

        // Reload
        let loaded = CompatLockFile::load(base_path).await.unwrap().unwrap();
        assert_eq!(loaded.negotiation.len(), 1);
        assert_eq!(loaded.negotiation[0].service_name, "user-service");
        assert!(loaded.is_sub_healthy());
    }

    #[tokio::test]
    async fn test_compat_lock_manager() {
        let temp_dir = TempDir::new().unwrap();
        // Use temp directory as project root
        let project_root = temp_dir.path().to_path_buf();

        let mut manager = CompatLockManager::new(project_root.clone());

        // Verify cache directory is under the system temp directory
        let cache_dir = manager.cache_dir().to_path_buf();
        assert!(cache_dir.starts_with(std::env::temp_dir()));
        assert!(cache_dir.to_string_lossy().contains("actr"));

        // Record a compatible match
        manager
            .record_negotiation(
                "user-service",
                "sha256:old",
                "sha256:new",
                false,
                CompatibilityCheck::BackwardCompatible,
            )
            .await
            .unwrap();

        // Verify file exists in the computed cache directory
        assert!(CompatLockFile::exists(&cache_dir).await);

        // Verify file is not in the project directory
        assert!(!project_root.join(COMPAT_LOCK_FILENAME).exists());

        // Find cached entry
        let entry = manager.find_cached_compatible("user-service", "sha256:old");
        assert!(entry.is_some());

        // After exact match, the entry should be removed
        manager
            .record_negotiation(
                "user-service",
                "sha256:exact",
                "sha256:exact",
                true,
                CompatibilityCheck::ExactMatch,
            )
            .await
            .unwrap();

        // File should be removed (no other entries remain)
        assert!(!CompatLockFile::exists(&cache_dir).await);
    }

    #[test]
    fn test_project_hash_deterministic() {
        let path1 = PathBuf::from("/tmp/test-project");
        let path2 = PathBuf::from("/tmp/test-project");
        let path3 = PathBuf::from("/tmp/other-project");

        let hash1 = compute_project_hash(&path1);
        let hash2 = compute_project_hash(&path2);
        let hash3 = compute_project_hash(&path3);

        // Same path should produce the same hash
        assert_eq!(hash1, hash2);
        // Different paths should produce different hashes
        assert_ne!(hash1, hash3);
        // Hash should be 16 hex characters
        assert_eq!(hash1.len(), 16);
    }
}
