//! Edition 1 configuration parser

use crate::config::ObservabilityConfig;
use crate::config::{
    ActrMode, Config, Dependency, IceServer, IceTransportPolicy, PackageInfo, ProtoFile,
    ServiceRef, WebRtcAdvancedConfig, WebRtcConfig,
};

use crate::error::{ConfigError, Result};
use crate::actr_raw::ActrRawConfig;
use crate::{RawConfig, RawDependency, RawPackageConfig, RawSystemConfig};
use actr_protocol::{Acl, ActrType, Name, Realm};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use url::Url;

const DEFAULT_TRACING_ENDPOINT: &str = "http://localhost:4317";

/// Edition 1 format parser
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
        // 1. Process inheritance
        let raw = if let Some(parent_path) = raw.inherit.take() {
            self.merge_inheritance(raw, parent_path)?
        } else {
            raw
        };

        // 2. Validate required fields
        self.validate_required_fields(&raw)?;

        // 3. Parse package
        let package = self.parse_package(&raw.package)?;

        // 4. Parse exports (prefer [package].exports, fallback to top-level exports)
        let export_paths = if !raw.package.exports.is_empty() {
            &raw.package.exports
        } else {
            &raw.exports
        };
        let exports = self.parse_exports(export_paths)?;

        // 5. Get realm
        let self_realm = Realm {
            realm_id: raw
                .system
                .deployment
                .realm_id
                .ok_or(ConfigError::MissingField("system.deployment.realm"))?,
        };

        // 5.1. Parse execution mode
        let execution_mode = parse_actr_mode(raw.system.deployment.mode.as_deref())?;

        // 6. Parse dependencies
        let dependencies = self.parse_dependencies(&raw.dependencies, &self_realm)?;

        // 7. Parse signaling URL
        let signaling_url_str = raw
            .system
            .signaling
            .url
            .as_ref()
            .ok_or(ConfigError::MissingField("system.signaling.url"))?;

        let signaling_url = Url::parse(signaling_url_str).map_err(ConfigError::InvalidUrl)?;
        let ais_endpoint = raw
            .system
            .ais_endpoint
            .url
            .as_ref()
            .ok_or(ConfigError::MissingField("system.ais_endpoint.url"))
            .and_then(|url| Url::parse(url).map_err(ConfigError::InvalidUrl))?;

        // 8. Parse observability config
        let observability = self.parse_observability(&raw.system, &package);

        // 10. Parse ACL (read from top-level acl, placed last to avoid partial move)
        let acl = if let Some(acl_value) = raw.acl {
            Some(self.parse_acl(acl_value, self_realm.realm_id)?)
        } else {
            None
        };

        // 11. Determine config_dir
        // If raw.config_dir exists, resolve it relative to the current base_dir
        let config_dir = if let Some(dir) = raw.config_dir {
            self.base_dir.join(dir)
        } else {
            self.base_dir.clone()
        };

        // 12. Build final config
        Ok(Config {
            package,
            exports,
            dependencies,
            signaling_url,
            realm: self_realm,
            realm_secret: raw.system.deployment.realm_secret.clone(),
            visible_in_discovery: raw.system.discovery.visible.unwrap_or(true),
            acl,
            mailbox_path: raw.system.storage.mailbox_path,
            tags: raw.package.tags,
            scripts: raw.scripts,
            webrtc: self.parse_webrtc(&raw.system.webrtc)?,
            websocket_listen_port: raw.system.websocket.listen_port,
            websocket_advertised_host: raw.system.websocket.advertised_host.clone(),
            observability,
            config_dir,
            execution_mode,
            ais_endpoint: Some(ais_endpoint.to_string()),
        })
    }

    /// Parse a `ActrRawConfig` (from actr.toml) into a `Config`.
    ///
    /// Since the runtime configuration has no `[package]` section, the caller must provide
    /// package info separately (typically read from actr.toml or .actr package).
    pub fn parse_actr(&self, raw: ActrRawConfig, package: PackageInfo, tags: Vec<String>) -> Result<Config> {
        // Validate required runtime fields
        let signaling_url_str = raw
            .signaling
            .url
            .as_ref()
            .ok_or(ConfigError::MissingField("signaling.url"))?;
        let signaling_url = Url::parse(signaling_url_str).map_err(ConfigError::InvalidUrl)?;

        let ais_endpoint = raw
            .ais_endpoint
            .url
            .as_ref()
            .ok_or(ConfigError::MissingField("ais_endpoint.url"))
            .and_then(|url| Url::parse(url).map_err(ConfigError::InvalidUrl))?;

        let self_realm = Realm {
            realm_id: raw
                .deployment
                .realm_id
                .ok_or(ConfigError::MissingField("deployment.realm_id"))?,
        };

        let execution_mode = parse_actr_mode(raw.deployment.mode.as_deref())?;

        // Build observability from flat config
        let observability = ObservabilityConfig {
            filter_level: raw
                .observability
                .filter_level
                .unwrap_or_else(|| "info".to_string()),
            tracing_enabled: raw.observability.tracing_enabled.unwrap_or(false),
            tracing_endpoint: raw
                .observability
                .tracing_endpoint
                .unwrap_or_else(|| DEFAULT_TRACING_ENDPOINT.to_string()),
            tracing_service_name: raw
                .observability
                .tracing_service_name
                .unwrap_or_else(|| package.name.clone()),
        };

        // Parse ACL
        let acl = if let Some(acl_value) = raw.acl {
            Some(self.parse_acl(acl_value, self_realm.realm_id)?)
        } else {
            None
        };

        Ok(Config {
            package,
            exports: vec![],       // runtime config has no exports
            dependencies: vec![],  // dependencies come from actr.toml
            signaling_url,
            realm: self_realm,
            realm_secret: raw.deployment.realm_secret,
            visible_in_discovery: raw.discovery.visible.unwrap_or(true),
            acl,
            mailbox_path: None,
            tags,
            scripts: raw.scripts,
            webrtc: self.parse_webrtc(&raw.webrtc)?,
            websocket_listen_port: raw.websocket.listen_port,
            websocket_advertised_host: raw.websocket.advertised_host,
            observability,
            config_dir: self.base_dir.clone(),
            execution_mode,
            ais_endpoint: Some(ais_endpoint.to_string()),
        })
    }

    fn parse_package(&self, raw: &RawPackageConfig) -> Result<PackageInfo> {
        Name::new(raw.manufacturer.clone()).map_err(|e| {
            ConfigError::InvalidActrType(format!(
                "Invalid manufacturer name '{}': {}",
                raw.manufacturer, e
            ))
        })?;

        Name::new(raw.name.clone()).map_err(|e| {
            ConfigError::InvalidActrType(format!("Invalid actor type name '{}': {}", raw.name, e))
        })?;

        let actr_type = ActrType {
            manufacturer: raw.manufacturer.clone(),
            name: raw.name.clone(),
            version: raw.version.clone(),
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
            .map(|(alias, raw_dep)| match raw_dep {
                RawDependency::Empty {} => Ok(Dependency {
                    alias: alias.clone(),
                    realm: *self_realm,
                    actr_type: None,
                    service: None,
                }),
                RawDependency::Specified {
                    actr_type: actr_type_str,
                    service,
                    realm,
                } => {
                    let actr_type = self.parse_actr_type(actr_type_str)?;
                    let service_ref = service
                        .as_deref()
                        .map(|s| self.parse_service_ref(s))
                        .transpose()?;
                    let realm = Realm {
                        realm_id: realm.unwrap_or(self_realm.realm_id),
                    };
                    Ok(Dependency {
                        alias: alias.clone(),
                        realm,
                        actr_type: Some(actr_type),
                        service: service_ref,
                    })
                }
            })
            .collect()
    }

    /// Parse `"ServiceName:fingerprint"` into a `ServiceRef`.
    fn parse_service_ref(&self, s: &str) -> Result<ServiceRef> {
        let (name, fingerprint) = s.split_once(':').ok_or_else(|| {
            ConfigError::InvalidActrType(format!(
                "Invalid service reference '{}': expected 'ServiceName:fingerprint'",
                s
            ))
        })?;
        Ok(ServiceRef {
            name: name.to_string(),
            fingerprint: fingerprint.to_string(),
        })
    }

    /// Parse an ActrType string: `"manufacturer:name:version"`.
    ///
    /// Note: `version` is required.
    fn parse_actr_type(&self, s: &str) -> Result<ActrType> {
        let parts: Vec<&str> = s.splitn(4, ':').collect();
        let (manufacturer, name, version) = match parts.as_slice() {
            [m, n, v] => (*m, *n, *v),
            _ => {
                return Err(ConfigError::InvalidActrType(format!(
                    "Invalid actor type '{}': expected 'manufacturer:name:version'",
                    s
                )));
            }
        };

        Name::new(manufacturer.to_string()).map_err(|e| {
            ConfigError::InvalidActrType(format!(
                "Invalid manufacturer '{}' in '{}': {}",
                manufacturer, s, e
            ))
        })?;
        Name::new(name.to_string()).map_err(|e| {
            ConfigError::InvalidActrType(format!("Invalid type name '{}' in '{}': {}", name, s, e))
        })?;
        if version.is_empty() {
            return Err(ConfigError::InvalidActrType(format!(
                "Invalid actor type '{}': version must not be empty",
                s
            )));
        }

        Ok(ActrType {
            manufacturer: manufacturer.to_string(),
            name: name.to_string(),
            version: version.to_string(),
        })
    }

    fn parse_acl(&self, value: toml::Value, self_realm_id: u32) -> Result<Acl> {
        use actr_protocol::AclRule;
        use actr_protocol::acl_rule::{Permission, SourceRealm};

        let rules_array = value
            .get("rules")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ConfigError::InvalidAcl("ACL must have 'rules' array".to_string()))?;

        let mut rules = Vec::new();

        for (idx, rule_value) in rules_array.iter().enumerate() {
            let rule_table = rule_value.as_table().ok_or_else(|| {
                ConfigError::InvalidAcl(format!("ACL rule {} must be a table", idx))
            })?;

            // permission (required)
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
                        "ACL rule {}: invalid permission '{}', expected 'ALLOW' or 'DENY'",
                        idx, permission_str
                    )));
                }
            };

            // type (required): "manufacturer:name:version"
            let type_str = rule_table
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ConfigError::InvalidAcl(format!("ACL rule {} missing 'type' field", idx))
                })?;
            let from_type = self.parse_actr_type(type_str)?;

            // realm (optional): omitted/"self" → self_realm_id, "*" → any, integer → specific
            let source_realm = match rule_table.get("realm") {
                None => Some(SourceRealm::RealmId(self_realm_id)),
                Some(v) => match v.as_str() {
                    Some("self") => Some(SourceRealm::RealmId(self_realm_id)),
                    Some("*") => Some(SourceRealm::AnyRealm(true)),
                    Some(other) => {
                        return Err(ConfigError::InvalidAcl(format!(
                            "ACL rule {}: invalid realm '{}', expected 'self', '*', or an integer",
                            idx, other
                        )));
                    }
                    None => match v.as_integer() {
                        Some(id) => Some(SourceRealm::RealmId(id as u32)),
                        None => {
                            return Err(ConfigError::InvalidAcl(format!(
                                "ACL rule {}: realm must be 'self', '*', or an integer",
                                idx
                            )));
                        }
                    },
                },
            };

            rules.push(AclRule {
                permission,
                from_type,
                source_realm,
            });
        }

        Ok(Acl { rules })
    }

    fn parse_webrtc(&self, raw: &crate::raw::RawWebRtcConfig) -> Result<WebRtcConfig> {
        let mut ice_servers = Vec::new();

        // Parse STUN URLs
        if !raw.stun_urls.is_empty() {
            ice_servers.push(IceServer {
                urls: raw.stun_urls.clone(),
                username: None,
                credential: None,
            });
        }

        // Parse TURN URLs (credentials are dynamically generated at runtime)
        if !raw.turn_urls.is_empty() {
            ice_servers.push(IceServer {
                urls: raw.turn_urls.clone(),
                username: None,
                credential: None,
            });
        }

        // Parse ICE transport policy
        let ice_transport_policy = if raw.force_relay {
            IceTransportPolicy::Relay
        } else {
            IceTransportPolicy::All
        };

        // Parse port configuration
        let (udp_ports, public_ips) =
            if let (Some(start), Some(end)) = (raw.port_range_start, raw.port_range_end) {
                // Port range configured, enable fixed port mode
                if start >= end {
                    // Invalid port range, return error
                    return Err(ConfigError::InvalidConfig(format!(
                        "Invalid port range: start ({}) must be less than end ({})",
                        start, end
                    )));
                } else {
                    // Port range mode
                    (Some((start, end)), raw.public_ips.clone())
                }
            } else {
                // No port range configured, use default mode (random ports)
                (None, Vec::new())
            };

        // Parse ICE acceptance wait times
        let ice_host_acceptance_min_wait = raw.ice_host_acceptance_min_wait.unwrap_or(0);
        let ice_srflx_acceptance_min_wait = raw.ice_srflx_acceptance_min_wait.unwrap_or(20);
        let ice_prflx_acceptance_min_wait = raw.ice_prflx_acceptance_min_wait.unwrap_or(40);
        let ice_relay_acceptance_min_wait = raw.ice_relay_acceptance_min_wait.unwrap_or(100);

        let advanced = WebRtcAdvancedConfig {
            udp_ports,
            public_ips,
            ice_host_acceptance_min_wait,
            ice_srflx_acceptance_min_wait,
            ice_prflx_acceptance_min_wait,
            ice_relay_acceptance_min_wait,
        };

        Ok(WebRtcConfig {
            ice_servers,
            ice_transport_policy,
            advanced,
        })
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

        // Check edition consistency
        if parent.edition != child.edition {
            return Err(ConfigError::EditionMismatch {
                parent: parent.edition,
                child: child.edition,
            });
        }

        // Recursively process parent config inheritance
        let parent = if let Some(grandparent) = parent.inherit.take() {
            self.merge_inheritance(parent, grandparent)?
        } else {
            parent
        };

        // Merge logic
        Ok(RawConfig {
            edition: child.edition, // Verified consistent
            inherit: None,
            config_dir: child.config_dir,
            package: child.package, // Package is not inherited
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
            ais_endpoint: crate::raw::RawAisEndpointConfig {
                url: child.ais_endpoint.url.or(parent.ais_endpoint.url),
            },
            deployment: crate::raw::RawDeploymentConfig {
                realm_id: child.deployment.realm_id.or(parent.deployment.realm_id),
                realm_secret: child
                    .deployment
                    .realm_secret
                    .or(parent.deployment.realm_secret),
                mode: child.deployment.mode.or(parent.deployment.mode),
                ais_endpoint: child
                    .deployment
                    .ais_endpoint
                    .or(parent.deployment.ais_endpoint),
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
                ice_host_acceptance_min_wait: child
                    .webrtc
                    .ice_host_acceptance_min_wait
                    .or(parent.webrtc.ice_host_acceptance_min_wait),
                ice_srflx_acceptance_min_wait: child
                    .webrtc
                    .ice_srflx_acceptance_min_wait
                    .or(parent.webrtc.ice_srflx_acceptance_min_wait),
                ice_prflx_acceptance_min_wait: child
                    .webrtc
                    .ice_prflx_acceptance_min_wait
                    .or(parent.webrtc.ice_prflx_acceptance_min_wait),
                ice_relay_acceptance_min_wait: child
                    .webrtc
                    .ice_relay_acceptance_min_wait
                    .or(parent.webrtc.ice_relay_acceptance_min_wait),
                port_range_start: child
                    .webrtc
                    .port_range_start
                    .or(parent.webrtc.port_range_start),
                port_range_end: child.webrtc.port_range_end.or(parent.webrtc.port_range_end),
                public_ips: if child.webrtc.public_ips.is_empty() {
                    parent.webrtc.public_ips
                } else {
                    child.webrtc.public_ips
                },
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
            websocket: crate::raw::RawWebSocketConfig {
                listen_port: child.websocket.listen_port.or(parent.websocket.listen_port),
                advertised_host: child
                    .websocket
                    .advertised_host
                    .or(parent.websocket.advertised_host),
            },
        }
    }

    fn validate_required_fields(&self, raw: &RawConfig) -> Result<()> {
        if raw.system.signaling.url.is_none() {
            return Err(ConfigError::MissingField("system.signaling.url"));
        }
        if raw.system.ais_endpoint.url.is_none() {
            return Err(ConfigError::MissingField("system.ais_endpoint.url"));
        }
        if raw.system.deployment.realm_id.is_none() {
            return Err(ConfigError::MissingField("system.deployment.realm"));
        }
        Ok(())
    }
}

/// Convert mode string from TOML to `ActrMode` enum
///
/// Valid values: `"native"` (default), `"process"`, `"wasm"`.
fn parse_actr_mode(s: Option<&str>) -> Result<ActrMode> {
    match s.unwrap_or("native") {
        "native" => Ok(ActrMode::Native),
        "process" => Ok(ActrMode::Process),
        "wasm" => Ok(ActrMode::Wasm),
        other => Err(ConfigError::InvalidConfig(format!(
            "system.deployment.mode value '{other}' is invalid, valid values are: native | process | wasm"
        ))),
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

[dependencies]
user-service = {}

[system.signaling]
url = "ws://localhost:8081"

[system.ais_endpoint]
url = "http://localhost:8081/ais"

[system.deployment]
realm_id = 1001

[scripts]
run = "cargo run"
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");

        // Create proto file
        let proto_dir = tmpdir.path().join("proto");
        fs::create_dir_all(&proto_dir).unwrap();
        fs::write(proto_dir.join("test.proto"), "syntax = \"proto3\";").unwrap();

        // Write config
        fs::write(&config_path, toml_content).unwrap();

        // Parse
        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse(raw).unwrap();

        assert_eq!(config.package.name, "test-service");
        assert_eq!(config.realm.realm_id, 1001);
        assert_eq!(config.dependencies.len(), 1);
        assert_eq!(config.exports.len(), 1);
        assert_eq!(
            config.ais_endpoint.as_deref(),
            Some("http://localhost:8081/ais")
        );
    }

    #[test]
    fn test_parse_cross_realm_dependency() {
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"
version = "1.0.0"

[dependencies]
shared = { actr_type = "acme:logging-service:1.0.0", service = "LoggingService:abc123", realm = 9999 }

[system.signaling]
url = "ws://localhost:8081"

[system.ais_endpoint]
url = "http://localhost:8081/ais"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse(raw).unwrap();

        let dep = config.get_dependency("shared").unwrap();
        assert_eq!(dep.alias, "shared");
        assert_eq!(dep.realm.realm_id, 9999);
        assert_eq!(dep.actr_type.as_ref().unwrap().name, "logging-service");
        assert_eq!(dep.service.as_ref().unwrap().name, "LoggingService");
        assert_eq!(dep.service.as_ref().unwrap().fingerprint, "abc123");
        assert!(dep.is_cross_realm(&config.realm));
        assert_eq!(
            config.ais_endpoint.as_deref(),
            Some("http://localhost:8081/ais")
        );
    }

    #[test]
    fn test_parse_explicit_ais_endpoint() {
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"

[system.signaling]
url = "ws://localhost:8081/signaling/ws"

[system.ais_endpoint]
url = "https://registry.example.com/custom-ais"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse(raw).unwrap();

        assert_eq!(
            config.ais_endpoint.as_deref(),
            Some("https://registry.example.com/custom-ais")
        );
    }

    #[test]
    fn test_missing_ais_endpoint_is_error() {
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"

[system.signaling]
url = "ws://localhost:8081/signaling/ws"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse(raw);

        assert!(matches!(
            result,
            Err(ConfigError::MissingField("system.ais_endpoint.url"))
        ));
    }

    #[test]
    fn test_validate_actr_type_name() {
        // Test invalid manufacturer name (starts with number)
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "1acme"

[system.signaling]
url = "ws://localhost:8081"

[system.ais_endpoint]
url = "http://localhost:8081/ais"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidActrType(_)
        ));
    }

    #[test]
    fn test_validate_actr_type_name_invalid() {
        // Test invalid actor type name (ends with hyphen)
        let toml_content = r#"
edition = 1

[package]
name = "test-"
manufacturer = "acme"

[system.signaling]
url = "ws://localhost:8081"

[system.ais_endpoint]
url = "http://localhost:8081/ais"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidActrType(_)
        ));
    }

    #[test]
    fn test_invalid_port_range() {
        // Test invalid port range (start > end)
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"

[system.signaling]
url = "ws://localhost:8081"

[system.ais_endpoint]
url = "http://localhost:8081/ais"

[system.deployment]
realm_id = 1001

[system.webrtc]
port_range_start = 50100
port_range_end = 50000
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse(raw);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::InvalidConfig(_)));
    }

    #[test]
    fn test_parse_execution_mode() {
        let base_toml = |mode_line: &str| {
            format!(
                r#"edition = 1
[package]
name = "test"
manufacturer = "acme"
[system.signaling]
url = "ws://localhost:8081"
[system.ais_endpoint]
url = "http://localhost:8081/ais"
[system.deployment]
realm_id = 1001
{mode_line}"#
            )
        };

        let tmpdir = TempDir::new().unwrap();

        // Default (no mode field) -> Native
        let path = tmpdir.path().join("actr.toml");
        fs::write(&path, base_toml("")).unwrap();
        let config = ParserV1::new(&path)
            .parse(RawConfig::from_file(&path).unwrap())
            .unwrap();
        assert_eq!(config.execution_mode, crate::config::ActrMode::Native);

        // mode = "process"
        fs::write(&path, base_toml("mode = \"process\"")).unwrap();
        let config = ParserV1::new(&path)
            .parse(RawConfig::from_file(&path).unwrap())
            .unwrap();
        assert_eq!(config.execution_mode, crate::config::ActrMode::Process);

        // mode = "wasm"
        fs::write(&path, base_toml("mode = \"wasm\"")).unwrap();
        let config = ParserV1::new(&path)
            .parse(RawConfig::from_file(&path).unwrap())
            .unwrap();
        assert_eq!(config.execution_mode, crate::config::ActrMode::Wasm);

        // Invalid value -> error
        fs::write(&path, base_toml("mode = \"invalid\"")).unwrap();
        let result = ParserV1::new(&path).parse(RawConfig::from_file(&path).unwrap());
        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
    }
}
