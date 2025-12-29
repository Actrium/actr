//! Lock file management for actr.lock.toml
//!
//! This module provides lock file structures with embedded proto content.
//! Unlike package managers like cargo/npm that cache large packages separately,
//! we embed proto content directly in the lock file because:
//! - Proto files are small (typically 2-10KB each)
//! - Total size is manageable (even 50 files = ~250KB)
//! - Simplifies architecture (single source of truth)
//! - Better for version control (can see proto changes in git diff)

use crate::error::{ConfigError, Result};
use actr_protocol::ServiceSpec;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;

/// Lock file structure for actr.lock.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LockFile {
    /// Lock file metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LockMetadata>,

    /// Locked dependencies (ordered array for deterministic output)
    #[serde(rename = "dependency")]
    pub dependencies: Vec<LockedDependency>,
}

/// Lock file metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMetadata {
    /// Lock file format version
    pub version: u32,
    /// Generation timestamp (ISO 8601)
    pub generated_at: String,
}

/// A locked dependency entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedDependency {
    /// Dependency name (matches Actr.toml key)
    pub name: String,

    /// Actor type (e.g., "acme+user-service")
    pub actr_type: String,

    /// Service specification (flattened)
    #[serde(flatten)]
    pub spec: ServiceSpecMeta,

    /// When this dependency was cached (ISO 8601)
    pub cached_at: String,
}

/// Service specification metadata for lock file
/// Contains complete proto content (not separated into cache)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpecMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Service-level semantic fingerprint
    pub fingerprint: String,

    /// Proto files with embedded content
    #[serde(rename = "files")]
    pub protobufs: Vec<ProtoFileWithContent>,

    /// Publication timestamp (Unix epoch seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,

    /// Tags like "latest", "stable"
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Package-level protobuf with embedded content
/// Note: Represents a merged package, not individual files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtoFileWithContent {
    /// Package name (e.g., "user.v1", "acme.payment.v2")
    /// Multiple .proto files of the same package are merged
    #[serde(rename = "package")]
    pub name: String,

    /// Semantic fingerprint of the merged package content
    pub fingerprint: String,

    /// Merged and normalized package content
    pub content: String,
}

// ============================================================================
// Bidirectional Conversion: ServiceSpec ↔ ServiceSpecMeta
// ============================================================================

impl From<ServiceSpec> for ServiceSpecMeta {
    fn from(spec: ServiceSpec) -> Self {
        Self {
            description: spec.description,
            fingerprint: spec.fingerprint,
            protobufs: spec
                .protobufs
                .into_iter()
                .map(|proto| ProtoFileWithContent {
                    name: proto.package,
                    fingerprint: proto.fingerprint,
                    content: proto.content,
                })
                .collect(),
            published_at: spec.published_at,
            tags: spec.tags,
        }
    }
}

impl From<ServiceSpecMeta> for ServiceSpec {
    fn from(meta: ServiceSpecMeta) -> Self {
        Self {
            description: meta.description,
            fingerprint: meta.fingerprint,
            protobufs: meta
                .protobufs
                .into_iter()
                .map(|proto| actr_protocol::service_spec::Protobuf {
                    package: proto.name, // ProtoFileWithContent.name → Protobuf.package
                    content: proto.content,
                    fingerprint: proto.fingerprint,
                })
                .collect(),
            published_at: meta.published_at,
            tags: meta.tags,
        }
    }
}

// ============================================================================
// LockFile Operations
// ============================================================================

impl LockFile {
    /// Create a new empty lock file with current timestamp
    pub fn new() -> Self {
        Self {
            metadata: Some(LockMetadata {
                version: 1,
                generated_at: chrono::Utc::now().to_rfc3339(),
            }),
            dependencies: Vec::new(),
        }
    }

    /// Load lock file from disk
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }

    /// Save lock file to disk
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Add or update a dependency
    pub fn add_dependency(&mut self, dep: LockedDependency) {
        // Remove existing entry with same name if exists
        self.dependencies.retain(|d| d.name != dep.name);

        // Add new entry
        self.dependencies.push(dep);

        // Sort by name for deterministic output
        self.dependencies.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Get a dependency by name
    pub fn get_dependency(&self, name: &str) -> Option<&LockedDependency> {
        self.dependencies.iter().find(|d| d.name == name)
    }

    /// Remove a dependency by name
    pub fn remove_dependency(&mut self, name: &str) -> bool {
        let before = self.dependencies.len();
        self.dependencies.retain(|d| d.name != name);
        self.dependencies.len() != before
    }

    /// Update generation timestamp
    pub fn update_timestamp(&mut self) {
        if let Some(ref mut metadata) = self.metadata {
            metadata.generated_at = chrono::Utc::now().to_rfc3339();
        }
    }
}

impl FromStr for LockFile {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(ConfigError::from)
    }
}

impl LockedDependency {
    /// Create a new locked dependency entry
    pub fn new(name: String, actr_type: String, spec: ServiceSpecMeta) -> Self {
        Self {
            name,
            actr_type,
            spec,
            cached_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Convert to ServiceSpec
    pub fn to_service_spec(&self) -> ServiceSpec {
        self.spec.clone().into()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_spec_conversion() {
        let spec = ServiceSpec {
            description: Some("Test service".to_string()),
            fingerprint: "service_semantic:abc123".to_string(),
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "user.v1".to_string(),
                content: "syntax = \"proto3\";".to_string(),
                fingerprint: "semantic:xyz".to_string(),
            }],
            published_at: Some(1705315800),
            tags: vec!["latest".to_string(), "stable".to_string()],
        };

        // Convert to meta
        let meta: ServiceSpecMeta = spec.clone().into();
        assert_eq!(meta.protobufs.len(), 1);
        assert_eq!(meta.protobufs[0].name, "user.v1");
        assert_eq!(meta.protobufs[0].content, "syntax = \"proto3\";");
        assert_eq!(meta.protobufs[0].fingerprint, "semantic:xyz");
        assert_eq!(meta.published_at, Some(1705315800));
        assert_eq!(meta.tags.len(), 2);

        // Convert back to ServiceSpec
        let restored: ServiceSpec = meta.into();
        assert_eq!(restored.fingerprint, spec.fingerprint);
        assert_eq!(restored.protobufs.len(), 1);
        assert_eq!(restored.protobufs[0].package, spec.protobufs[0].package);
        assert_eq!(restored.protobufs[0].content, spec.protobufs[0].content);
    }

    #[test]
    fn test_lock_file_operations() {
        let mut lock_file = LockFile::new();
        assert!(lock_file.dependencies.is_empty());

        let spec_meta = ServiceSpecMeta {
            description: None,
            fingerprint: "service_semantic:test".to_string(),
            protobufs: vec![],
            published_at: None,
            tags: vec![],
        };

        let dep = LockedDependency::new(
            "test-service".to_string(),
            "acme+test-service".to_string(),
            spec_meta,
        );

        lock_file.add_dependency(dep);
        assert_eq!(lock_file.dependencies.len(), 1);

        let found = lock_file.get_dependency("test-service");
        assert!(found.is_some());
        assert_eq!(found.unwrap().actr_type, "acme+test-service");

        let removed = lock_file.remove_dependency("test-service");
        assert!(removed);
        assert!(lock_file.dependencies.is_empty());
    }

    #[test]
    fn test_lock_file_serialization() {
        let mut lock_file = LockFile::new();

        let spec_meta = ServiceSpecMeta {
            description: Some("User service".to_string()),
            fingerprint: "service_semantic:abc123".to_string(),
            protobufs: vec![ProtoFileWithContent {
                name: "user.v1".to_string(),
                fingerprint: "semantic:xyz".to_string(),
                content: "syntax = \"proto3\";\n\npackage user.v1;".to_string(),
            }],
            published_at: Some(1705315800),
            tags: vec!["latest".to_string()],
        };

        let dep = LockedDependency::new(
            "user-service".to_string(),
            "acme+user-service".to_string(),
            spec_meta,
        );

        lock_file.add_dependency(dep);

        // Serialize to TOML
        let toml_str = toml::to_string_pretty(&lock_file).unwrap();
        assert!(toml_str.contains("user-service"));
        assert!(toml_str.contains("syntax = \"proto3\""));
        assert!(toml_str.contains("service_semantic:abc123"));

        // Deserialize back
        let restored: LockFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.dependencies.len(), 1);
        assert_eq!(restored.dependencies[0].name, "user-service");
        assert_eq!(
            restored.dependencies[0].spec.protobufs[0].content,
            "syntax = \"proto3\";\n\npackage user.v1;"
        );
    }

    #[test]
    fn test_multiline_proto_content() {
        let content = r#"syntax = "proto3";

package user.v1;

message User {
  uint64 id = 1;
  string name = 2;
}

service UserService {
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
}
"#;

        let spec_meta = ServiceSpecMeta {
            description: None,
            fingerprint: "service_semantic:test".to_string(),
            protobufs: vec![ProtoFileWithContent {
                name: "user.v1".to_string(),
                fingerprint: "semantic:abc".to_string(),
                content: content.to_string(),
            }],
            published_at: None,
            tags: vec![],
        };

        // Serialize
        let toml_str = toml::to_string_pretty(&spec_meta).unwrap();

        // Should use multiline string
        assert!(toml_str.contains("content = '''") || toml_str.contains("content = \"\"\""));

        // Deserialize back
        let restored: ServiceSpecMeta = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.protobufs[0].content, content);
    }
}
