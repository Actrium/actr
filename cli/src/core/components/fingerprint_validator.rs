//! Default FingerprintValidator implementation

use anyhow::Result;
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use walkdir::WalkDir;

use super::{Fingerprint, FingerprintValidator, ResolvedDependency, ServiceInfo};

/// Default fingerprint validator
pub struct DefaultFingerprintValidator;

impl DefaultFingerprintValidator {
    pub fn new() -> Self {
        Self
    }

    /// Compute SHA256 of a file's content
    fn hash_file(path: &Path) -> Result<Vec<u8>> {
        let mut hasher = Sha256::new();
        let mut file = std::fs::File::open(path)?;
        let mut buffer = [0u8; 8192];

        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }

        Ok(hasher.finalize().to_vec())
    }
}

impl Default for DefaultFingerprintValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FingerprintValidator for DefaultFingerprintValidator {
    async fn compute_service_fingerprint(&self, service: &ServiceInfo) -> Result<Fingerprint> {
        Ok(Fingerprint {
            algorithm: "sha256".to_string(),
            value: service.fingerprint.clone(),
        })
    }

    async fn verify_fingerprint(
        &self,
        expected: &Fingerprint,
        actual: &Fingerprint,
    ) -> Result<bool> {
        Ok(expected.algorithm == actual.algorithm && expected.value == actual.value)
    }

    async fn compute_project_fingerprint(&self, project_path: &Path) -> Result<Fingerprint> {
        let mut hasher = Sha256::new();
        let mut proto_files: Vec<_> = WalkDir::new(project_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("proto"))
            .collect();

        // Sort files to ensure deterministic hash
        proto_files.sort_by(|a, b| a.path().cmp(b.path()));

        for entry in proto_files {
            let file_hash = Self::hash_file(entry.path())?;
            hasher.update(&file_hash);
        }

        Ok(Fingerprint {
            algorithm: "sha256".to_string(),
            value: hex::encode(hasher.finalize()),
        })
    }

    async fn generate_lock_fingerprint(&self, deps: &[ResolvedDependency]) -> Result<Fingerprint> {
        let mut hasher = Sha256::new();
        let mut dep_names: Vec<_> = deps.iter().map(|d| &d.spec.name).collect();
        dep_names.sort();

        for name in dep_names {
            hasher.update(name.as_bytes());
            if let Some(dep) = deps.iter().find(|d| d.spec.name == *name) {
                hasher.update(dep.fingerprint.as_bytes());
            }
        }

        Ok(Fingerprint {
            algorithm: "sha256".to_string(),
            value: hex::encode(hasher.finalize()),
        })
    }
}
