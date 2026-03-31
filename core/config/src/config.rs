//! Final configuration structures - fully parsed and validated

use actr_protocol::{Acl, ActrType, Realm};
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

/// Manifest configuration — parsed from `manifest.toml`.
///
/// Carries workload package metadata, proto exports, dependencies, ACL, and scripts.
/// Does **not** contain runtime fields like `signaling_url`, `realm`, or `ais_endpoint`
/// — those belong to [`RuntimeConfig`] parsed from `actr.toml`.
#[derive(Debug, Clone)]
pub struct ManifestConfig {
    /// Package info
    pub package: PackageInfo,

    /// Exported proto files (contents loaded)
    pub exports: Vec<ProtoFile>,

    /// Service dependencies (expanded)
    pub dependencies: Vec<Dependency>,

    /// Access control list
    pub acl: Option<Acl>,

    /// Service tags (e.g., "latest", "stable", "v1.0")
    pub tags: Vec<String>,

    /// Script commands
    pub scripts: HashMap<String, String>,

    /// Directory containing `manifest.toml`
    pub config_dir: PathBuf,
}

/// Runtime configuration — parsed from `actr.toml`.
///
/// Carries all deployment and networking settings needed by the actor runtime.
/// Required fields (`signaling_url`, `realm`, `ais_endpoint`) are **non-Option**;
/// the parser validates their presence before construction.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Package info (provided by caller, e.g. from the .actr package or lock file)
    pub package: PackageInfo,

    // ── Required runtime fields (non-Option) ──
    /// Signaling server URL (validated)
    pub signaling_url: Url,

    /// Owning Realm (Security Realm)
    pub realm: Realm,

    /// AIS (Actor Identity Service) HTTP endpoint
    pub ais_endpoint: String,

    // ── Optional runtime fields ──
    /// Realm secret for AIS registration authentication
    pub realm_secret: Option<String>,

    /// Whether visible in service discovery
    pub visible_in_discovery: bool,

    /// Access control list
    pub acl: Option<Acl>,

    /// Mailbox database path (`None` → in-memory mode)
    pub mailbox_path: Option<PathBuf>,

    /// Service tags
    pub tags: Vec<String>,

    /// Script commands
    pub scripts: HashMap<String, String>,

    /// WebRTC configuration
    pub webrtc: WebRtcConfig,

    /// Port for listening to inbound WebSocket connections (direct mode)
    pub websocket_listen_port: Option<u16>,

    /// WebSocket hostname or IP advertised to the signaling server
    pub websocket_advertised_host: Option<String>,

    /// Observability configuration (logging + tracing)
    pub observability: ObservabilityConfig,

    /// Directory containing `actr.toml`
    pub config_dir: PathBuf,

    /// Hyper data directory (.hyper)
    pub hyper_data_dir: PathBuf,

    /// Trust mode: "development" or "production"
    pub trust_mode: String,

    /// Path to the workload package (.actr file)
    pub package_path: Option<PathBuf>,
}

/// Package info
#[derive(Debug, Clone)]
pub struct PackageInfo {
    /// Package name
    pub name: String,

    /// Actor type
    pub actr_type: ActrType,

    /// Description
    pub description: Option<String>,

    /// Author list
    pub authors: Vec<String>,

    /// License
    pub license: Option<String>,
}

/// Parsed proto file (file level)
#[derive(Debug, Clone)]
pub struct ProtoFile {
    /// File path (absolute)
    pub path: PathBuf,

    /// File content
    pub content: String,
}

/// Expanded dependency
#[derive(Debug, Clone)]
pub struct Dependency {
    /// Dependency alias (key in dependencies)
    pub alias: String,

    /// Owning Realm
    pub realm: Realm,

    /// Actor type (manufacturer:name:version)
    pub actr_type: Option<ActrType>,

    /// Strict service reference for exact fingerprint matching.
    /// Parsed from `service = "ServiceName:fingerprint"`.
    pub service: Option<ServiceRef>,
}

/// Strict service reference: proto service name + semantic fingerprint.
///
/// When present on a dependency, the runtime only connects to service
/// instances whose registered fingerprint exactly matches.
#[derive(Debug, Clone)]
pub struct ServiceRef {
    /// Proto service name (e.g., "EchoService")
    pub name: String,
    /// Proto semantic fingerprint (e.g., "abc1f3d")
    pub fingerprint: String,
}

/// ICE transport policy
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum IceTransportPolicy {
    /// Use all available candidates (default)
    #[default]
    All,
    /// Use only TURN relay candidates
    Relay,
}

/// ICE server configuration
#[derive(Clone, Debug, Default)]
pub struct IceServer {
    /// Server URL list
    pub urls: Vec<String>,
    /// Username (required for TURN servers)
    pub username: Option<String>,
    /// Credential (required for TURN servers)
    pub credential: Option<String>,
}

/// UDP port configuration
type UdpPorts = Option<(u16, u16)>;

/// WebRTC advanced parameter configuration
#[derive(Clone, Debug)]
pub struct WebRtcAdvancedConfig {
    /// UDP port policy
    pub udp_ports: UdpPorts,
    /// NAT 1:1 public IP mapping
    pub public_ips: Vec<String>,
    /// ICE host candidate acceptance min wait (ms)
    pub ice_host_acceptance_min_wait: u64,
    /// ICE srflx candidate acceptance min wait (ms)
    pub ice_srflx_acceptance_min_wait: u64,
    /// ICE prflx candidate acceptance min wait (ms)
    pub ice_prflx_acceptance_min_wait: u64,
    /// ICE relay candidate acceptance min wait (ms)
    pub ice_relay_acceptance_min_wait: u64,
}

impl WebRtcAdvancedConfig {
    /// Check whether advanced parameters are configured and prefer being Answerer
    pub fn prefer_answerer(&self) -> bool {
        // If port range or public_ips are configured, prefer being Answerer
        self.udp_ports.is_some() || !self.public_ips.is_empty()
    }
}

impl Default for WebRtcAdvancedConfig {
    fn default() -> Self {
        Self {
            udp_ports: UdpPorts::default(),
            public_ips: Vec::new(),
            ice_host_acceptance_min_wait: 0,
            ice_srflx_acceptance_min_wait: 20,
            ice_prflx_acceptance_min_wait: 40,
            ice_relay_acceptance_min_wait: 100,
        }
    }
}

/// WebRTC configuration
#[derive(Clone, Debug, Default)]
pub struct WebRtcConfig {
    /// ICE server list
    pub ice_servers: Vec<IceServer>,
    /// ICE transport policy (All or Relay)
    pub ice_transport_policy: IceTransportPolicy,
    /// Advanced parameter configuration
    pub advanced: WebRtcAdvancedConfig,
}
/// Observability configuration (logging + tracing) resolved from raw config
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Filter level (e.g., "info", "debug", "warn", "info,webrtc=debug").
    /// Used when RUST_LOG environment variable is not set. Default: "info".
    pub filter_level: String,

    /// Whether to enable distributed tracing
    pub tracing_enabled: bool,

    /// OTLP/Jaeger gRPC endpoint
    pub tracing_endpoint: String,

    /// Service name reported to the tracing backend
    pub tracing_service_name: String,
}

// ============================================================================
// ManifestConfig helper methods
// ============================================================================

impl ManifestConfig {
    /// Get the package's ActrType
    pub fn actr_type(&self) -> &ActrType {
        &self.package.actr_type
    }

    /// Get all proto file paths
    pub fn proto_paths(&self) -> Vec<&PathBuf> {
        self.exports.iter().map(|p| &p.path).collect()
    }

    /// Get all proto contents (for computing service fingerprint)
    pub fn proto_contents(&self) -> Vec<&str> {
        self.exports.iter().map(|p| p.content.as_str()).collect()
    }

    /// Find a dependency by alias
    pub fn get_dependency(&self, alias: &str) -> Option<&Dependency> {
        self.dependencies.iter().find(|d| d.alias == alias)
    }

    /// Get a script command
    pub fn get_script(&self, name: &str) -> Option<&str> {
        self.scripts.get(name).map(|s| s.as_str())
    }

    /// List all script names
    pub fn list_scripts(&self) -> Vec<&str> {
        self.scripts.keys().map(|s| s.as_str()).collect()
    }

    /// Calculate ServiceSpec from manifest
    ///
    /// Returns None if no proto files are exported
    pub fn calculate_service_spec(&self) -> Option<actr_protocol::ServiceSpec> {
        if self.exports.is_empty() {
            return None;
        }

        let proto_files: Vec<actr_service_compat::ProtoFile> = self
            .exports
            .iter()
            .map(|export| actr_service_compat::ProtoFile {
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

        let fingerprint =
            actr_service_compat::Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
                .ok()?;

        let protobufs = self
            .exports
            .iter()
            .map(|export| {
                let file_fingerprint =
                    actr_service_compat::Fingerprint::calculate_proto_semantic_fingerprint(
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

        let published_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;

        Some(actr_protocol::ServiceSpec {
            name: self.package.name.clone(),
            description: self.package.description.clone(),
            fingerprint,
            protobufs,
            published_at: Some(published_at),
            tags: self.tags.clone(),
        })
    }
}

// ============================================================================
// RuntimeConfig helper methods
// ============================================================================

impl RuntimeConfig {
    /// Get the package's ActrType (for registration)
    pub fn actr_type(&self) -> &ActrType {
        &self.package.actr_type
    }

    /// Get all cross-Realm dependencies.
    ///
    /// Returns an empty vec (runtime config does not carry dependencies).
    pub fn cross_realm_dependencies(&self) -> Vec<&Dependency> {
        // RuntimeConfig does not have dependencies field
        vec![]
    }

    /// Get a script command
    pub fn get_script(&self, name: &str) -> Option<&str> {
        self.scripts.get(name).map(|s| s.as_str())
    }

    /// Calculate ServiceSpec from runtime config.
    ///
    /// Returns None — runtime config does not carry proto exports.
    /// Use `ManifestConfig::calculate_service_spec()` instead.
    pub fn calculate_service_spec(&self) -> Option<actr_protocol::ServiceSpec> {
        None
    }
}

// ============================================================================
// PackageInfo helper methods
// ============================================================================

impl PackageInfo {
    /// Get manufacturer (ActrType.manufacturer)
    pub fn manufacturer(&self) -> &str {
        &self.actr_type.manufacturer
    }

    /// Get type name (ActrType.name)
    pub fn type_name(&self) -> &str {
        &self.actr_type.name
    }
}

// ============================================================================
// Dependency helper methods
// ============================================================================

impl Dependency {
    /// Whether this is a cross-Realm dependency
    pub fn is_cross_realm(&self, self_realm: &Realm) -> bool {
        self.realm.realm_id != self_realm.realm_id
    }

    /// Check whether exact fingerprint matching is required (i.e., `service` field exists)
    pub fn requires_exact_fingerprint(&self) -> bool {
        self.service.is_some()
    }

    /// Check whether the fingerprint matches
    ///
    /// - No `service` field: always matches (loose dependency)
    /// - Has `service` field: must match exactly
    pub fn matches_fingerprint(&self, fingerprint: &str) -> bool {
        self.service
            .as_ref()
            .map(|s| s.fingerprint == fingerprint)
            .unwrap_or(true)
    }
}

// ============================================================================
// ProtoFile helper methods
// ============================================================================

impl ProtoFile {
    /// Get file name
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name()?.to_str()
    }

    /// Get file extension
    pub fn extension(&self) -> Option<&str> {
        self.path.extension()?.to_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_config_methods() {
        let config = ManifestConfig {
            package: PackageInfo {
                name: "test-service".to_string(),
                actr_type: ActrType {
                    manufacturer: "acme".to_string(),
                    name: "test-service".to_string(),
                    version: "1.0.0".to_string(),
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
                    actr_type: Some(ActrType {
                        manufacturer: "acme".to_string(),
                        name: "user-service".to_string(),
                        version: "2.1.0".to_string(),
                    }),
                    service: Some(ServiceRef {
                        name: "UserService".to_string(),
                        fingerprint: "abc123".to_string(),
                    }),
                },
                Dependency {
                    alias: "shared-logger".to_string(),
                    realm: Realm { realm_id: 9999 },
                    actr_type: Some(ActrType {
                        manufacturer: "common".to_string(),
                        name: "logging-service".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    service: None,
                },
            ],
            acl: None,
            tags: vec![],
            scripts: HashMap::new(),
            config_dir: PathBuf::from("."),
        };

        // Test dependency lookup
        assert!(config.get_dependency("user-service").is_some());
        assert!(config.get_dependency("not-exists").is_none());

        // Test fingerprint matching
        let user_dep = config.get_dependency("user-service").unwrap();
        assert!(user_dep.matches_fingerprint("abc123"));
        assert!(!user_dep.matches_fingerprint("different"));

        let logger_dep = config.get_dependency("shared-logger").unwrap();
        assert!(logger_dep.matches_fingerprint("any-fingerprint"));
        assert!(!logger_dep.requires_exact_fingerprint());
    }

    #[test]
    fn test_runtime_config_methods() {
        let config = RuntimeConfig {
            package: PackageInfo {
                name: "test-service".to_string(),
                actr_type: ActrType {
                    manufacturer: "acme".to_string(),
                    name: "test-service".to_string(),
                    version: "1.0.0".to_string(),
                },
                description: None,
                authors: vec![],
                license: None,
            },
            signaling_url: Url::parse("ws://localhost:8081").unwrap(),
            realm: Realm { realm_id: 1001 },
            ais_endpoint: "http://localhost:8081/ais".to_string(),
            realm_secret: None,
            visible_in_discovery: true,
            acl: None,
            mailbox_path: None,
            tags: vec![],
            scripts: HashMap::new(),
            webrtc: WebRtcConfig::default(),
            websocket_listen_port: None,
            websocket_advertised_host: None,
            observability: ObservabilityConfig {
                filter_level: "info".to_string(),
                tracing_enabled: false,
                tracing_endpoint: "http://localhost:4317".to_string(),
                tracing_service_name: "test-service".to_string(),
            },
            config_dir: PathBuf::from("."),
            hyper_data_dir: PathBuf::from(".hyper"),
            trust_mode: "development".to_string(),
            package_path: None,
        };

        assert_eq!(config.actr_type().name, "test-service");
        assert!(config.cross_realm_dependencies().is_empty());
        assert!(config.calculate_service_spec().is_none());
    }
}
