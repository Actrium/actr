//! Default CacheManager implementation
//!
//! Proto files are cached to the project's `protos/remote/` folder (not ~/.actr/cache)
//! following the documentation spec for dependency management.

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

use super::{CacheManager, CacheStats, CachedProto, Fingerprint, ProtoFile};

/// Default cache manager (file-based, project-local)
///
/// Caches proto files to `{project_root}/protos/remote/{service_name}/` directory
/// following the documentation spec.
pub struct DefaultCacheManager {
    /// Project root directory (where manifest.toml is located)
    project_root: PathBuf,
}

impl DefaultCacheManager {
    pub fn new() -> Self {
        Self {
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    pub fn with_project_root(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    /// Get the proto cache directory for a service
    /// Returns: {project_root}/protos/remote/{service_name}/
    fn get_service_proto_dir(&self, service_name: &str) -> PathBuf {
        self.project_root
            .join("protos")
            .join("remote")
            .join(service_name)
    }
}

impl Default for DefaultCacheManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CacheManager for DefaultCacheManager {
    async fn get_cached_proto(&self, service_name: &str) -> Result<Option<CachedProto>> {
        let cache_path = self.get_service_proto_dir(service_name);

        if !cache_path.exists() {
            return Ok(None);
        }

        let mut files = Vec::new();
        for entry in std::fs::read_dir(&cache_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "proto").unwrap_or(false) {
                let content = std::fs::read_to_string(&path)?;
                files.push(ProtoFile {
                    name: path.file_name().unwrap().to_string_lossy().to_string(),
                    path,
                    content,
                    services: Vec::new(),
                });
            }
        }

        if files.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CachedProto {
                files,
                fingerprint: Fingerprint {
                    algorithm: "sha256".to_string(),
                    value: "cached".to_string(),
                },
                cached_at: std::time::SystemTime::now(),
                expires_at: None,
            }))
        }
    }

    async fn cache_proto(&self, service_name: &str, files: &[ProtoFile]) -> Result<()> {
        let cache_path = self.get_service_proto_dir(service_name);
        std::fs::create_dir_all(&cache_path)?;

        for file in files {
            // Use the proto file name directly (e.g., echo.v1.proto)
            let file_name = if file.name.ends_with(".proto") {
                file.name.clone()
            } else {
                format!("{}.proto", file.name)
            };
            let file_path = cache_path.join(&file_name);
            std::fs::write(&file_path, &file.content)?;
            tracing::debug!(
                "Cached proto file: {} -> {}",
                file.name,
                file_path.display()
            );
        }

        tracing::info!(
            "Cached {} proto files to protos/remote/{}/",
            files.len(),
            service_name
        );
        Ok(())
    }

    async fn invalidate_cache(&self, service_name: &str) -> Result<()> {
        let cache_path = self.get_service_proto_dir(service_name);
        if cache_path.exists() {
            std::fs::remove_dir_all(&cache_path)?;
        }
        Ok(())
    }

    async fn clear_cache(&self) -> Result<()> {
        let proto_dir = self.project_root.join("protos");
        if proto_dir.exists() {
            std::fs::remove_dir_all(&proto_dir)?;
        }
        Ok(())
    }

    async fn get_cache_stats(&self) -> Result<CacheStats> {
        let proto_dir = self.project_root.join("protos");
        let mut total_size = 0u64;
        let mut entry_count = 0usize;

        if proto_dir.exists() {
            for entry in std::fs::read_dir(&proto_dir)? {
                entry_count += 1;
                let entry = entry?;
                if entry.path().is_dir() {
                    for file in std::fs::read_dir(entry.path())? {
                        let file = file?;
                        total_size += file.metadata()?.len();
                    }
                }
            }
        }

        Ok(CacheStats {
            total_entries: entry_count,
            total_size_bytes: total_size,
            hit_rate: 0.0,
            miss_rate: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn get_cached_proto_returns_none_when_path_absent() {
        let dir = TempDir::new().unwrap();
        let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
        assert!(mgr.get_cached_proto("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cache_get_invalidate_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());

        // Cache a proto file.
        let proto = ProtoFile {
            name: "echo.proto".into(),
            path: "echo.proto".into(),
            content: "syntax = \"proto3\";".into(),
            services: vec![],
        };
        mgr.cache_proto("echo", &[proto]).await.unwrap();

        // Now retrieve it.
        let cached = mgr.get_cached_proto("echo").await.unwrap().unwrap();
        assert_eq!(cached.files.len(), 1);
        assert_eq!(cached.files[0].name, "echo.proto");
        assert!(!cached.fingerprint.value.is_empty());

        // Invalidate.
        mgr.invalidate_cache("echo").await.unwrap();
        assert!(mgr.get_cached_proto("echo").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cache_proto_adds_dot_proto_extension_when_missing() {
        let dir = TempDir::new().unwrap();
        let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
        let proto = ProtoFile {
            name: "echo".into(),
            path: "echo".into(),
            content: "syntax = \"proto3\";".into(),
            services: vec![],
        };
        mgr.cache_proto("echo", &[proto]).await.unwrap();
        let cached = mgr.get_cached_proto("echo").await.unwrap().unwrap();
        // The name field is unchanged, but the file on disk has .proto appended.
        assert_eq!(cached.files.len(), 1);
        assert!(cached.files[0].name.ends_with(".proto"));
    }

    #[tokio::test]
    async fn clear_cache_removes_protos_directory() {
        let dir = TempDir::new().unwrap();
        let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
        let proto = ProtoFile {
            name: "echo.proto".into(),
            path: "echo.proto".into(),
            content: "// stub".into(),
            services: vec![],
        };
        mgr.cache_proto("echo", &[proto]).await.unwrap();
        assert!(dir.path().join("protos").exists());

        mgr.clear_cache().await.unwrap();
        assert!(!dir.path().join("protos").exists());
    }

    #[tokio::test]
    async fn get_cache_stats_counts_entries_and_sizes() {
        let dir = TempDir::new().unwrap();
        let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
        let proto = ProtoFile {
            name: "echo.proto".into(),
            path: "echo.proto".into(),
            content: "// hi".into(),
            services: vec![],
        };
        mgr.cache_proto("echo", &[proto]).await.unwrap();
        let stats = mgr.get_cache_stats().await.unwrap();
        assert_eq!(stats.total_entries, 1);
        assert!(stats.total_size_bytes > 0);
        assert_eq!(stats.hit_rate, 0.0);
        assert_eq!(stats.miss_rate, 0.0);
    }
}
