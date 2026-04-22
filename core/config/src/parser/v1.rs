//! Edition 1 configuration parser

use crate::config::ObservabilityConfig;
use crate::config::{
    BinaryConfig, BuildArtifact, BuildConfig, BuildProfile, BuildTool, Dependency, IceServer,
    IceTransportPolicy, ManifestConfig, PackageInfo, ProtoFile, RuntimeConfig, ServiceRef,
    TrustAnchor, WebRtcAdvancedConfig, WebRtcConfig,
};

use crate::actr_raw::RuntimeRawConfig;
use crate::error::{ConfigError, Result};
use crate::{RawBuildConfig, ManifestRawConfig, RawDependency, RawPackageConfig, WebConfig};
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

    pub fn parse_manifest(&self, mut raw: ManifestRawConfig) -> Result<ManifestConfig> {
        // 1. Process inheritance
        let raw = if let Some(parent_path) = raw.inherit.take() {
            self.merge_inheritance(raw, parent_path)?
        } else {
            raw
        };

        // 2. Parse package
        let package = self.parse_package(&raw.package)?;

        // 3. Parse exports (prefer [package].exports, fallback to top-level exports)
        let export_paths = if !raw.package.exports.is_empty() {
            &raw.package.exports
        } else {
            &raw.exports
        };
        let exports = self.parse_exports(export_paths)?;

        // 4. Parse dependencies with a placeholder realm (realm comes from actr.toml at runtime)
        //    For dependencies with no explicit realm override, they will use realm_id 0 as a
        //    placeholder; the runtime resolves the actual realm from actr.toml.
        let placeholder_realm = Realm { realm_id: 0 };
        let dependencies = self.parse_dependencies(&raw.dependencies, &placeholder_realm)?;

        // 5. Parse ACL
        let acl = if let Some(acl_value) = raw.acl {
            // Use realm_id 0 as placeholder since realm is not known at manifest parse time
            Some(self.parse_acl(acl_value, 0)?)
        } else {
            None
        };

        // 6. Determine config_dir
        let config_dir = if let Some(dir) = raw.config_dir {
            self.base_dir.join(dir)
        } else {
            self.base_dir.clone()
        };

        let binary = raw
            .binary
            .as_ref()
            .map(|raw_binary| self.parse_binary(raw_binary));
        let build = raw
            .build
            .as_ref()
            .map(|raw_build| self.parse_build(raw_build))
            .transpose()?;

        if raw.build.is_some() && raw.binary.is_none() {
            return Err(ConfigError::InvalidConfig(
                "[build] requires [binary] to declare the final packaged artifact path".to_string(),
            ));
        }

        // 7. Build manifest config — runtime fields are NOT included
        Ok(ManifestConfig {
            package,
            exports,
            dependencies,
            acl,
            tags: raw.package.tags,
            scripts: raw.scripts,
            binary,
            build,
            config_dir,
        })
    }

    /// Parse a `RuntimeRawConfig` (from actr.toml) into a `RuntimeConfig`.
    ///
    /// Since the runtime configuration has no `[package]` section, the caller must provide
    /// package info separately (typically read from the .actr package).
    pub fn parse_runtime(
        &self,
        raw: RuntimeRawConfig,
        package: PackageInfo,
    ) -> Result<RuntimeConfig> {
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

        let package_path = raw
            .package
            .and_then(|pkg| pkg.path)
            .map(|path| self.resolve_manifest_path(&path));

        Ok(RuntimeConfig {
            package,
            signaling_url,
            realm: self_realm,
            ais_endpoint: ais_endpoint.to_string(),
            realm_secret: raw.deployment.realm_secret,
            visible_in_discovery: raw.discovery.visible.unwrap_or(true),
            acl,
            mailbox_path: None,
            scripts: raw.scripts,
            webrtc: self.parse_webrtc(&raw.webrtc)?,
            websocket_listen_port: raw.websocket.listen_port,
            websocket_advertised_host: raw.websocket.advertised_host,
            observability,
            config_dir: self.base_dir.clone(),
            trust: raw
                .trust
                .into_iter()
                .map(|anchor| resolve_trust_paths(anchor, &self.base_dir))
                .collect(),
            package_path,
            web: raw.web.map(|w| WebConfig {
                port: w.port,
                host: w.host,
                static_dir: self.base_dir.join(&w.static_dir),
                package_url: w.package_url,
                runtime_wasm_url: w.runtime_wasm_url,
            }),
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

    fn parse_binary(&self, raw: &crate::RawBinaryConfig) -> BinaryConfig {
        BinaryConfig {
            path: self.resolve_manifest_path(&raw.path),
            target: raw.target.clone(),
        }
    }

    fn parse_build(&self, raw: &RawBuildConfig) -> Result<BuildConfig> {
        let tool = match raw.tool.as_deref().unwrap_or("cargo") {
            "cargo" => BuildTool::Cargo,
            other => {
                return Err(ConfigError::InvalidConfig(format!(
                    "Unsupported [build].tool '{other}'; v1 only supports 'cargo'"
                )));
            }
        };

        let artifact = match raw.artifact.as_deref().unwrap_or("lib") {
            "lib" => BuildArtifact::Lib,
            "bin" => BuildArtifact::Bin,
            other => {
                return Err(ConfigError::InvalidConfig(format!(
                    "Unsupported [build].artifact '{other}'; expected 'lib' or 'bin'"
                )));
            }
        };

        let profile = match raw.profile.as_deref().unwrap_or("release") {
            "release" => BuildProfile::Release,
            "dev" => BuildProfile::Dev,
            other => {
                return Err(ConfigError::InvalidConfig(format!(
                    "Unsupported [build].profile '{other}'; expected 'release' or 'dev'"
                )));
            }
        };

        Ok(BuildConfig {
            tool,
            manifest_path: self.resolve_manifest_path(
                raw.manifest_path
                    .as_deref()
                    .unwrap_or_else(|| Path::new("Cargo.toml")),
            ),
            artifact,
            target: raw.target.clone(),
            profile,
            features: raw.features.clone(),
            no_default_features: raw.no_default_features,
            post_build: raw.post_build.clone(),
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

    fn resolve_manifest_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        }
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
                        realm_id: realm.unwrap_or(1),
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

    fn merge_inheritance(&self, child: ManifestRawConfig, parent_path: PathBuf) -> Result<ManifestRawConfig> {
        let parent_full_path = self.base_dir.join(&parent_path);
        let mut parent = ManifestRawConfig::from_file(&parent_full_path)?;

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

        // Merge logic — system config is no longer part of manifest.toml
        Ok(ManifestRawConfig {
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
            acl: child.acl.or(parent.acl),
            scripts: {
                let mut s = parent.scripts;
                s.extend(child.scripts);
                s
            },
            binary: child.binary.or(parent.binary),
            build: child.build.or(parent.build),
        })
    }
}

// (ActrMode removed) previous execution mode parsing no longer needed.

/// Resolve a trust anchor's relative paths against the config dir.
fn resolve_trust_paths(anchor: TrustAnchor, base_dir: &Path) -> TrustAnchor {
    match anchor {
        TrustAnchor::Static {
            pubkey_file,
            pubkey_b64,
        } => TrustAnchor::Static {
            pubkey_file: pubkey_file.map(|p| if p.is_absolute() { p } else { base_dir.join(p) }),
            pubkey_b64,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ManifestRawConfig;
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

[scripts]
run = "cargo run"
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("manifest.toml");

        // Create proto file
        let proto_dir = tmpdir.path().join("proto");
        fs::create_dir_all(&proto_dir).unwrap();
        fs::write(proto_dir.join("test.proto"), "syntax = \"proto3\";").unwrap();

        // Write config
        fs::write(&config_path, toml_content).unwrap();

        // Parse
        let raw = ManifestRawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse_manifest(raw).unwrap();

        assert_eq!(config.package.name, "test-service");
        // ManifestConfig carries only manifest-level fields
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
version = "1.0.0"

[dependencies]
shared = { actr_type = "acme:logging-service:1.0.0", service = "LoggingService:abc123", realm = 9999 }
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("manifest.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = ManifestRawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse_manifest(raw).unwrap();

        let dep = config.get_dependency("shared").unwrap();
        assert_eq!(dep.alias, "shared");
        assert_eq!(dep.realm.realm_id, 9999);
        assert_eq!(dep.actr_type.as_ref().unwrap().name, "logging-service");
        assert_eq!(dep.service.as_ref().unwrap().name, "LoggingService");
        assert_eq!(dep.service.as_ref().unwrap().fingerprint, "abc123");
        // ManifestConfig does not carry runtime fields (realm, signaling_url, ais_endpoint)
        // cross_realm_dependencies() is a ManifestConfig method based on dependency realm IDs
        assert!(!config.dependencies.is_empty());
    }

    #[test]
    fn test_parse_package_fields() {
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("manifest.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = ManifestRawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse_manifest(raw).unwrap();

        assert_eq!(config.package.name, "test");
        assert_eq!(config.package.actr_type.manufacturer, "acme");
        // ManifestConfig does not carry runtime fields (realm, signaling_url, ais_endpoint)
    }

    #[test]
    fn test_validate_actr_type_name() {
        // Test invalid manufacturer name (starts with number)
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "1acme"
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("manifest.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = ManifestRawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse_manifest(raw);
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
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("manifest.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = ManifestRawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse_manifest(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidActrType(_)
        ));
    }

    #[test]
    fn test_parse_binary_and_build_config() {
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"
version = "1.0.0"

[binary]
path = "dist/test.wasm"
target = "wasm32-unknown-unknown"

[build]
tool = "cargo"
manifest_path = "Cargo.toml"
artifact = "lib"
profile = "release"
target = "wasm32-unknown-unknown"
features = ["feature-a", "feature-b"]
no_default_features = true
post_build = ["echo build"]
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("manifest.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = ManifestRawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse_manifest(raw).unwrap();

        let binary = config.binary.expect("binary config should exist");
        assert_eq!(binary.path, tmpdir.path().join("dist/test.wasm"));
        assert_eq!(binary.target.as_deref(), Some("wasm32-unknown-unknown"));

        let build = config.build.expect("build config should exist");
        assert_eq!(build.manifest_path, tmpdir.path().join("Cargo.toml"));
        assert_eq!(build.artifact, BuildArtifact::Lib);
        assert_eq!(build.profile, BuildProfile::Release);
        assert_eq!(build.features, vec!["feature-a", "feature-b"]);
        assert!(build.no_default_features);
        assert_eq!(build.post_build, vec!["echo build"]);
    }

    #[test]
    fn test_build_requires_binary_config() {
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"

[build]
tool = "cargo"
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("manifest.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = ManifestRawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse_manifest(raw);

        assert!(matches!(result, Err(ConfigError::InvalidConfig(_))));
    }

    #[test]
    fn test_parse_execution_mode_not_in_manifest() {
        // manifest.toml does not carry execution mode — that belongs to RuntimeConfig (actr.toml).
        // Verify that parsing manifest.toml succeeds and produces a valid ManifestConfig.
        let toml_content = r#"
edition = 1

[package]
name = "test"
manufacturer = "acme"
"#;

        let tmpdir = TempDir::new().unwrap();
        let path = tmpdir.path().join("manifest.toml");
        fs::write(&path, toml_content).unwrap();

        let config = ParserV1::new(&path)
            .parse_manifest(ManifestRawConfig::from_file(&path).unwrap())
            .unwrap();
        // ManifestConfig fields are present; no execution_mode field exists on ManifestConfig
        assert_eq!(config.package.name, "test");
        assert_eq!(config.package.actr_type.manufacturer, "acme");
    }
}
