//! Final configuration structures - fully parsed and validated

use actr_protocol::{Acl, ActrType, Realm};
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

/// 最终配置（已处理继承、默认值、验证、类型转换）
/// 注意：没有 edition 字段，edition 只作用于解析阶段
#[derive(Debug, Clone)]
pub struct Config {
    /// 包信息
    pub package: PackageInfo,

    /// 导出的 proto 文件（已读取内容）
    pub exports: Vec<ProtoFile>,

    /// 服务依赖（已展开）
    pub dependencies: Vec<Dependency>,

    /// 信令服务器 URL（已验证）
    pub signaling_url: Url,

    /// 所属 Realm (Security Realm)
    pub realm: Realm,

    /// 是否在服务发现中可见
    pub visible_in_discovery: bool,

    /// 访问控制列表
    pub acl: Option<Acl>,

    /// Mailbox 数据库路径
    ///
    /// - `Some(path)`: 使用持久化 SQLite 数据库
    /// - `None`: 使用内存模式 (`:memory:`)
    pub mailbox_path: Option<PathBuf>,

    /// Service tags (e.g., "latest", "stable", "v1.0")
    pub tags: Vec<String>,

    /// 脚本命令
    pub scripts: HashMap<String, String>,

    /// Observability configuration (logging + tracing)
    pub tracing: ObservabilityConfig,
}

/// 包信息
#[derive(Debug, Clone)]
pub struct PackageInfo {
    /// 包名
    pub name: String,

    /// Actor 类型
    pub actr_type: ActrType,

    /// 描述
    pub description: Option<String>,

    /// 作者列表
    pub authors: Vec<String>,

    /// 许可证
    pub license: Option<String>,
}

/// 已解析的 proto 文件（文件级别）
#[derive(Debug, Clone)]
pub struct ProtoFile {
    /// 文件路径（绝对路径）
    pub path: PathBuf,

    /// 文件内容
    pub content: String,
}

/// 已展开的依赖
#[derive(Debug, Clone)]
pub struct Dependency {
    /// 依赖别名（dependencies 中的 key）
    pub alias: String,

    /// 所属 Realm
    pub realm: Realm,

    /// Actor 类型
    pub actr_type: ActrType,

    /// 服务指纹
    pub fingerprint: Option<String>,
}

/// Observability configuration (logging + tracing) resolved from raw config
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Log level (e.g., "info", "debug", "warn").
    /// Used when RUST_LOG environment variable is not set. Default: "info".
    pub log_level: String,

    /// Whether to enable distributed tracing
    pub enabled: bool,

    /// OTLP/Jaeger gRPC endpoint
    pub endpoint: String,

    /// Service name reported to the tracing backend
    pub service_name: String,
}

// ============================================================================
// Config 辅助方法
// ============================================================================

impl Config {
    /// 获取包的 ActrType（用于注册）
    pub fn actr_type(&self) -> &ActrType {
        &self.package.actr_type
    }

    /// 获取所有 proto 文件路径
    pub fn proto_paths(&self) -> Vec<&PathBuf> {
        self.exports.iter().map(|p| &p.path).collect()
    }

    /// 获取所有 proto 内容（用于计算服务指纹）
    pub fn proto_contents(&self) -> Vec<&str> {
        self.exports.iter().map(|p| p.content.as_str()).collect()
    }

    /// 根据别名查找依赖
    pub fn get_dependency(&self, alias: &str) -> Option<&Dependency> {
        self.dependencies.iter().find(|d| d.alias == alias)
    }

    /// 根据 ActrType 查找所有匹配的依赖
    pub fn find_dependencies_by_type(&self, actr_type: &ActrType) -> Vec<&Dependency> {
        self.dependencies
            .iter()
            .filter(|d| &d.actr_type == actr_type)
            .collect()
    }

    /// 获取所有跨 Realm 的依赖
    pub fn cross_realm_dependencies(&self) -> Vec<&Dependency> {
        self.dependencies
            .iter()
            .filter(|d| d.realm.realm_id != self.realm.realm_id)
            .collect()
    }

    /// 获取脚本命令
    pub fn get_script(&self, name: &str) -> Option<&str> {
        self.scripts.get(name).map(|s| s.as_str())
    }

    /// 列出所有脚本名称
    pub fn list_scripts(&self) -> Vec<&str> {
        self.scripts.keys().map(|s| s.as_str()).collect()
    }

    /// Calculate ServiceSpec from config
    ///
    /// Returns None if no proto files are exported
    pub fn calculate_service_spec(&self) -> Option<actr_protocol::ServiceSpec> {
        // If no exports, no ServiceSpec
        if self.exports.is_empty() {
            return None;
        }

        // Convert exports to ProtoFile format for fingerprint calculation
        let proto_files: Vec<actr_version::ProtoFile> = self
            .exports
            .iter()
            .map(|export| actr_version::ProtoFile {
                name: export
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown.proto")
                    .to_string(),
                content: export.content.clone(),
                path: export.path.to_str().map(|s| s.to_string()),
            })
            .collect();

        // Calculate service fingerprint
        let fingerprint =
            actr_version::Fingerprint::calculate_service_semantic_fingerprint(&proto_files).ok()?;

        // Build Protobuf entries
        let protobufs = self
            .exports
            .iter()
            .map(|export| {
                // Calculate individual file fingerprint
                let file_fingerprint =
                    actr_version::Fingerprint::calculate_proto_semantic_fingerprint(
                        &export.content,
                    )
                    .unwrap_or_else(|_| "error".to_string());

                actr_protocol::service_spec::Protobuf {
                    package: export
                        .path
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    content: export.content.clone(),
                    fingerprint: file_fingerprint,
                }
            })
            .collect();

        // Get current timestamp
        let published_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;

        Some(actr_protocol::ServiceSpec {
            description: self.package.description.clone(),
            fingerprint,
            protobufs,
            published_at: Some(published_at),
            tags: self.tags.clone(),
        })
    }
}

// ============================================================================
// PackageInfo 辅助方法
// ============================================================================

impl PackageInfo {
    /// 获取 manufacturer（ActrType.manufacturer）
    pub fn manufacturer(&self) -> &str {
        &self.actr_type.manufacturer
    }

    /// 获取 type name（ActrType.name）
    pub fn type_name(&self) -> &str {
        &self.actr_type.name
    }
}

// ============================================================================
// Dependency 辅助方法
// ============================================================================

impl Dependency {
    /// 是否跨 Realm 依赖
    pub fn is_cross_realm(&self, self_realm: &Realm) -> bool {
        self.realm.realm_id != self_realm.realm_id
    }

    /// 检查指纹是否匹配
    pub fn matches_fingerprint(&self, fingerprint: &str) -> bool {
        self.fingerprint
            .as_ref()
            .map(|fp| fp == fingerprint)
            .unwrap_or(true) // 无指纹要求则总是匹配
    }
}

// ============================================================================
// ProtoFile 辅助方法
// ============================================================================

impl ProtoFile {
    /// 获取文件名
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name()?.to_str()
    }

    /// 获取文件扩展名
    pub fn extension(&self) -> Option<&str> {
        self.path.extension()?.to_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_methods() {
        let config = Config {
            package: PackageInfo {
                name: "test-service".to_string(),
                actr_type: ActrType {
                    manufacturer: "acme".to_string(),
                    name: "test-service".to_string(),
                },
                description: None,
                authors: vec![],
                license: None,
            },
            exports: vec![],
            dependencies: vec![
                Dependency {
                    alias: "user-service".to_string(),
                    realm: Realm { realm_id: 1001 },
                    actr_type: ActrType {
                        manufacturer: "acme".to_string(),
                        name: "user-service".to_string(),
                    },
                    fingerprint: Some("service_semantic:abc123...".to_string()),
                },
                Dependency {
                    alias: "shared-logger".to_string(),
                    realm: Realm { realm_id: 9999 },
                    actr_type: ActrType {
                        manufacturer: "common".to_string(),
                        name: "logging-service".to_string(),
                    },
                    fingerprint: None,
                },
            ],
            signaling_url: Url::parse("ws://localhost:8081").unwrap(),
            realm: Realm { realm_id: 1001 },
            visible_in_discovery: true,
            acl: None,
            mailbox_path: None,
            tags: vec![],
            scripts: HashMap::new(),
            tracing: ObservabilityConfig {
                log_level: "info".to_string(),
                enabled: false,
                endpoint: "http://localhost:4317".to_string(),
                service_name: "test-service".to_string(),
            },
        };

        // 测试依赖查找
        assert!(config.get_dependency("user-service").is_some());
        assert!(config.get_dependency("not-exists").is_none());

        // 测试跨 Realm 依赖
        let cross_realm = config.cross_realm_dependencies();
        assert_eq!(cross_realm.len(), 1);
        assert_eq!(cross_realm[0].alias, "shared-logger");

        // 测试指纹匹配
        let user_dep = config.get_dependency("user-service").unwrap();
        assert!(user_dep.matches_fingerprint("service_semantic:abc123..."));
        assert!(!user_dep.matches_fingerprint("service_semantic:different"));
    }
}
