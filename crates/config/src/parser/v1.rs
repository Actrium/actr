//! Edition 1 configuration parser

use crate::config::ObservabilityConfig;
use crate::config::{
    Config, Dependency, IceServer, IceTransportPolicy, PackageInfo, ProtoFile, WebRtcConfig,
};

use crate::error::{ConfigError, Result};
use crate::{RawConfig, RawDependency, RawPackageConfig, RawSystemConfig};
use actr_protocol::{Acl, ActrType, Realm};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use url::Url;

const DEFAULT_TRACING_ENDPOINT: &str = "http://localhost:4317";

/// Edition 1 格式的解析器
pub struct ParserV1 {
    base_dir: PathBuf,
}

impl ParserV1 {
    pub fn new(config_path: impl AsRef<Path>) -> Self {
        let base_dir = config_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self { base_dir }
    }

    pub fn parse(&self, mut raw: RawConfig) -> Result<Config> {
        // 1. 处理继承
        let raw = if let Some(parent_path) = raw.inherit.take() {
            self.merge_inheritance(raw, parent_path)?
        } else {
            raw
        };

        // 2. 验证必需字段
        self.validate_required_fields(&raw)?;

        // 3. 解析 package
        let package = self.parse_package(&raw.package)?;

        // 4. 解析 exports
        let exports = self.parse_exports(&raw.exports)?;

        // 5. 获取 realm
        let self_realm = Realm {
            realm_id: raw
                .system
                .deployment
                .realm
                .ok_or(ConfigError::MissingField("system.deployment.realm"))?,
        };

        // 6. 解析 dependencies
        let dependencies = self.parse_dependencies(&raw.dependencies, &self_realm)?;

        // 7. 解析 signaling URL
        let signaling_url_str = raw
            .system
            .signaling
            .url
            .as_ref()
            .ok_or(ConfigError::MissingField("system.signaling.url"))?;

        let signaling_url = Url::parse(signaling_url_str).map_err(ConfigError::InvalidUrl)?;

        // 8. 解析 WebRTC 配置
        let webrtc = self.parse_webrtc(&raw.system.webrtc);

        // 9. 解析 observability 配置
        let observability = self.parse_observability(&raw.system, &package);

        // 10. 解析 ACL (从顶级 acl 读取，放在最后以避免 partial move)
        let acl = if let Some(acl_value) = raw.acl {
            Some(self.parse_acl(acl_value)?)
        } else {
            None
        };

        // 11. 构建最终配置
        Ok(Config {
            package,
            exports,
            dependencies,
            signaling_url,
            realm: self_realm,
            visible_in_discovery: raw.system.discovery.visible.unwrap_or(true),
            acl,
            mailbox_path: raw.system.storage.mailbox_path,
            tags: raw.package.tags,
            scripts: raw.scripts,
            webrtc,
            observability,
        })
    }

    fn parse_package(&self, raw: &RawPackageConfig) -> Result<PackageInfo> {
        let actr_type = ActrType {
            manufacturer: raw.manufacturer.clone(),
            name: raw.type_name.clone(),
        };

        Ok(PackageInfo {
            name: raw.name.clone(),
            actr_type,
            description: raw.description.clone(),
            authors: raw.authors.clone().unwrap_or_default(),
            license: raw.license.clone(),
        })
    }

    fn parse_exports(&self, paths: &[PathBuf]) -> Result<Vec<ProtoFile>> {
        paths
            .iter()
            .map(|path| {
                let full_path = self.base_dir.join(path);
                let content = std::fs::read_to_string(&full_path)
                    .map_err(|e| ConfigError::ProtoFileNotFound(full_path.clone(), e))?;
                Ok(ProtoFile {
                    path: full_path,
                    content,
                })
            })
            .collect()
    }

    fn parse_dependencies(
        &self,
        deps: &HashMap<String, RawDependency>,
        self_realm: &Realm,
    ) -> Result<Vec<Dependency>> {
        deps.iter()
            .map(|(alias, raw_dep)| {
                let (realm_id, actr_type_str, fingerprint) = match raw_dep {
                    RawDependency::Empty {} => {
                        // 空依赖声明：使用别名作为 actr_type，无指纹
                        (None, alias.clone(), None)
                    }
                    RawDependency::WithFingerprint {
                        realm,
                        actr_type,
                        fingerprint,
                    } => {
                        let type_str = actr_type.as_ref().unwrap_or(alias).clone();
                        (*realm, type_str, Some(fingerprint.clone()))
                    }
                };

                // 解析 ActrType
                let actr_type = self.parse_actr_type(&actr_type_str)?;

                // 确定 realm
                let realm = Realm {
                    realm_id: realm_id.unwrap_or(self_realm.realm_id),
                };

                Ok(Dependency {
                    alias: alias.clone(),
                    realm,
                    actr_type,
                    fingerprint,
                })
            })
            .collect()
    }

    fn parse_actr_type(&self, s: &str) -> Result<ActrType> {
        // 支持两种格式：
        // 1. "service-name" -> manufacturer = "", name = "service-name"
        // 2. "manufacturer:service-name"
        if let Some((manufacturer, name)) = s.split_once(':') {
            Ok(ActrType {
                manufacturer: manufacturer.to_string(),
                name: name.to_string(),
            })
        } else {
            Ok(ActrType {
                manufacturer: String::new(),
                name: s.to_string(),
            })
        }
    }

    fn parse_acl(&self, value: toml::Value) -> Result<Acl> {
        use actr_protocol::AclRule;
        use actr_protocol::acl_rule::{Permission, Principal};

        // Parse [[acl.rules]] array
        let rules_array = value
            .get("rules")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ConfigError::InvalidAcl("ACL must have 'rules' array".to_string()))?;

        let mut rules = Vec::new();

        for (idx, rule_value) in rules_array.iter().enumerate() {
            let rule_table = rule_value.as_table().ok_or_else(|| {
                ConfigError::InvalidAcl(format!("ACL rule {} must be a table", idx))
            })?;

            // Parse permission (required)
            let permission_str = rule_table
                .get("permission")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ConfigError::InvalidAcl(format!("ACL rule {} missing 'permission' field", idx))
                })?;

            let permission = match permission_str.to_uppercase().as_str() {
                "ALLOW" => Permission::Allow as i32,
                "DENY" => Permission::Deny as i32,
                _ => {
                    return Err(ConfigError::InvalidAcl(format!(
                        "Invalid permission '{}' in ACL rule {}, expected 'ALLOW' or 'DENY'",
                        permission_str, idx
                    )));
                }
            };

            // Parse principals - support both old 'principals' and new 'types' format
            let principals = if let Some(types_value) = rule_table.get("types") {
                // New format: types = ["acme:allowed-client", "acme:admin"]
                let types_array = types_value.as_array().ok_or_else(|| {
                    ConfigError::InvalidAcl(format!("ACL rule {} 'types' must be an array", idx))
                })?;

                let mut principals_list = Vec::new();
                for (t_idx, type_value) in types_array.iter().enumerate() {
                    let type_str = type_value.as_str().ok_or_else(|| {
                        ConfigError::InvalidAcl(format!(
                            "ACL rule {} type {} must be a string",
                            idx, t_idx
                        ))
                    })?;

                    // Parse "manufacturer:name" format
                    let actr_type = self.parse_actr_type(type_str)?;

                    principals_list.push(Principal {
                        realm: None, // No realm specified in types format
                        actr_type: Some(actr_type),
                    });
                }
                principals_list
            } else if let Some(principals_value) = rule_table.get("principals") {
                // Old format: principals = [{ realm = 0, actr_type = ... }]
                let principals_array = principals_value.as_array().ok_or_else(|| {
                    ConfigError::InvalidAcl(format!(
                        "ACL rule {} 'principals' must be an array",
                        idx
                    ))
                })?;

                let mut principals_list = Vec::new();
                for (p_idx, principal_value) in principals_array.iter().enumerate() {
                    let principal_table = principal_value.as_table().ok_or_else(|| {
                        ConfigError::InvalidAcl(format!(
                            "ACL rule {} principal {} must be a table",
                            idx, p_idx
                        ))
                    })?;

                    // Parse optional realm
                    let realm = if let Some(realm_value) = principal_table.get("realm") {
                        let realm_id = realm_value.as_integer().ok_or_else(|| {
                            ConfigError::InvalidAcl(format!(
                                "ACL rule {} principal {} realm must be an integer",
                                idx, p_idx
                            ))
                        })? as u32;
                        Some(actr_protocol::Realm { realm_id })
                    } else {
                        None
                    };

                    // Parse optional actr_type
                    let actr_type = if let Some(type_value) = principal_table.get("actr_type") {
                        if let Some(type_str) = type_value.as_str() {
                            Some(self.parse_actr_type(type_str)?)
                        } else if let Some(type_table) = type_value.as_table() {
                            // Support inline table: { manufacturer = "acme", name = "service" }
                            let manufacturer = type_table
                                .get("manufacturer")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = type_table
                                .get("name")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    ConfigError::InvalidAcl(format!(
                                        "ACL rule {} principal {} actr_type must have 'name' field",
                                        idx, p_idx
                                    ))
                                })?
                                .to_string();
                            Some(ActrType { manufacturer, name })
                        } else {
                            return Err(ConfigError::InvalidAcl(format!(
                                "ACL rule {} principal {} actr_type must be a string or table",
                                idx, p_idx
                            )));
                        }
                    } else {
                        None
                    };

                    principals_list.push(Principal { realm, actr_type });
                }
                principals_list
            } else {
                Vec::new()
            };

            rules.push(AclRule {
                principals,
                permission,
            });
        }

        Ok(Acl { rules })
    }

    fn parse_webrtc(&self, raw: &crate::raw::RawWebRtcConfig) -> WebRtcConfig {
        let mut ice_servers = Vec::new();

        // 解析 STUN URLs
        if !raw.stun_urls.is_empty() {
            ice_servers.push(IceServer {
                urls: raw.stun_urls.clone(),
                username: None,
                credential: None,
            });
        }

        // 解析 TURN URLs（凭证在运行时动态生成）
        if !raw.turn_urls.is_empty() {
            ice_servers.push(IceServer {
                urls: raw.turn_urls.clone(),
                username: None,
                credential: None,
            });
        }

        // 解析 ICE 传输策略
        let ice_transport_policy = if raw.force_relay {
            IceTransportPolicy::Relay
        } else {
            IceTransportPolicy::All
        };

        WebRtcConfig {
            ice_servers,
            ice_transport_policy,
        }
    }
    fn parse_observability(
        &self,
        raw_system: &RawSystemConfig,
        package: &PackageInfo,
    ) -> ObservabilityConfig {
        ObservabilityConfig {
            filter_level: raw_system
                .observability
                .filter_level
                .clone()
                .unwrap_or_else(|| "info".to_string()),
            tracing_enabled: raw_system.observability.tracing_enabled.unwrap_or(false),
            tracing_endpoint: raw_system
                .observability
                .tracing_endpoint
                .clone()
                .unwrap_or_else(|| DEFAULT_TRACING_ENDPOINT.to_string()),
            tracing_service_name: raw_system
                .observability
                .tracing_service_name
                .clone()
                .unwrap_or_else(|| package.name.clone()),
        }
    }

    fn merge_inheritance(&self, child: RawConfig, parent_path: PathBuf) -> Result<RawConfig> {
        let parent_full_path = self.base_dir.join(&parent_path);
        let mut parent = RawConfig::from_file(&parent_full_path)?;

        // 检查 edition 一致性
        if parent.edition != child.edition {
            return Err(ConfigError::EditionMismatch {
                parent: parent.edition,
                child: child.edition,
            });
        }

        // 递归处理父配置的继承
        let parent = if let Some(grandparent) = parent.inherit.take() {
            self.merge_inheritance(parent, grandparent)?
        } else {
            parent
        };

        // 合并逻辑
        Ok(RawConfig {
            edition: child.edition, // 已验证一致
            inherit: None,
            package: child.package, // package 不继承
            exports: {
                let mut p = parent.exports;
                p.extend(child.exports);
                p
            },
            dependencies: {
                let mut d = parent.dependencies;
                d.extend(child.dependencies);
                d
            },
            system: self.merge_system_config(parent.system, child.system),
            acl: child.acl.or(parent.acl),
            scripts: {
                let mut s = parent.scripts;
                s.extend(child.scripts);
                s
            },
        })
    }

    fn merge_system_config(
        &self,
        parent: RawSystemConfig,
        child: RawSystemConfig,
    ) -> RawSystemConfig {
        RawSystemConfig {
            signaling: crate::raw::RawSignalingConfig {
                url: child.signaling.url.or(parent.signaling.url),
            },
            deployment: crate::raw::RawDeploymentConfig {
                realm: child.deployment.realm.or(parent.deployment.realm),
            },
            discovery: crate::raw::RawDiscoveryConfig {
                visible: child.discovery.visible.or(parent.discovery.visible),
            },
            storage: crate::raw::RawStorageConfig {
                mailbox_path: child.storage.mailbox_path.or(parent.storage.mailbox_path),
            },
            webrtc: crate::raw::RawWebRtcConfig {
                stun_urls: if child.webrtc.stun_urls.is_empty() {
                    parent.webrtc.stun_urls
                } else {
                    child.webrtc.stun_urls
                },
                turn_urls: if child.webrtc.turn_urls.is_empty() {
                    parent.webrtc.turn_urls
                } else {
                    child.webrtc.turn_urls
                },
                force_relay: child.webrtc.force_relay || parent.webrtc.force_relay,
            },
            observability: crate::raw::RawObservabilityConfig {
                filter_level: child
                    .observability
                    .filter_level
                    .or(parent.observability.filter_level.clone()),
                tracing_enabled: child
                    .observability
                    .tracing_enabled
                    .or(parent.observability.tracing_enabled),
                tracing_endpoint: child
                    .observability
                    .tracing_endpoint
                    .or(parent.observability.tracing_endpoint),
                tracing_service_name: child
                    .observability
                    .tracing_service_name
                    .or(parent.observability.tracing_service_name),
            },
        }
    }

    fn validate_required_fields(&self, raw: &RawConfig) -> Result<()> {
        if raw.system.signaling.url.is_none() {
            return Err(ConfigError::MissingField("system.signaling.url"));
        }
        if raw.system.deployment.realm.is_none() {
            return Err(ConfigError::MissingField("system.deployment.realm"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawConfig;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_basic_config() {
        let toml_content = r#"
edition = 1
exports = ["proto/test.proto"]

[package]
name = "test-service"
manufacturer = "acme"
type = "test-service"

[dependencies]
user-service = {}

[system.signaling]
url = "ws://localhost:8081"

[system.deployment]
realm = 1001

[scripts]
run = "cargo run"
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("Actr.toml");

        // 创建 proto 文件
        let proto_dir = tmpdir.path().join("proto");
        fs::create_dir_all(&proto_dir).unwrap();
        fs::write(proto_dir.join("test.proto"), "syntax = \"proto3\";").unwrap();

        // 写入配置
        fs::write(&config_path, toml_content).unwrap();

        // 解析
        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse(raw).unwrap();

        assert_eq!(config.package.name, "test-service");
        assert_eq!(config.realm.realm_id, 1001);
        assert_eq!(config.dependencies.len(), 1);
        assert_eq!(config.exports.len(), 1);
    }

    #[test]
    fn test_parse_cross_realm_dependency() {
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"
type = "test"

[dependencies]
shared = { actr_type = "logging-service", realm = 9999, fingerprint = "service_semantic:abc123..." }

[system.signaling]
url = "ws://localhost:8081"

[system.deployment]
realm = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("Actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse(raw).unwrap();

        let dep = config.get_dependency("shared").unwrap();
        assert_eq!(dep.realm.realm_id, 9999);
        assert_eq!(dep.actr_type.name, "logging-service");
        assert!(dep.is_cross_realm(&config.realm));
    }
}
