//! Final configuration structures - fully parsed and validated

use actr_protocol::{Acl, ActrType, Realm};
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

/// Actor execution mode
///
/// Determines how the actor runtime obtains credentials and cooperates with the Hyper host layer.
/// Specified via the `mode` field in `[system.deployment]` section of the runtime `actr.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ActrMode {
    /// Native process mode (default)
    ///
    /// The actor runtime runs in a standalone process, registers with the signaling server
    /// and obtains credentials on its own.
    /// Suitable for development/debugging or deployments not managed by Hyper.
    #[default]
    Native,

    /// Subprocess mode
    ///
    /// The actor runtime is launched as a subprocess by the Hyper layer.
    /// Credentials are injected by Hyper via `inject_credential()` or
    /// `ACTR_REGISTER_OK` environment variable; skips signaling registration
    /// at startup and directly uses the issued credentials.
    Process,

    /// WASM module mode
    ///
    /// The actor runtime runs as a WASM module inside the Hyper host.
    /// Credentials are injected via host functions or `inject_credential()` (TBD).
    Wasm,
}

impl std::fmt::Display for ActrMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActrMode::Native => write!(f, "native"),
            ActrMode::Process => write!(f, "process"),
            ActrMode::Wasm => write!(f, "wasm"),
        }
    }
}

/// Final config (inheritance, defaults, validation, and type conversion applied)
/// Note: no edition field -- edition only affects the parsing phase
#[derive(Debug, Clone)]
pub struct Config {
    /// Package info
    pub package: PackageInfo,

    /// Exported proto files (contents loaded)
    pub exports: Vec<ProtoFile>,

    /// Service dependencies (expanded)
    pub dependencies: Vec<Dependency>,

    /// Signaling server URL (validated)
    pub signaling_url: Url,

    /// Owning Realm (Security Realm)
    pub realm: Realm,

    /// Realm secret for AIS registration authentication (optional)
    ///
    /// Required when the realm has secret-based access control enabled.
    pub realm_secret: Option<String>,

    /// Whether visible in service discovery
    pub visible_in_discovery: bool,

    /// Access control list
    pub acl: Option<Acl>,

    /// Mailbox database path
    ///
    /// - `Some(path)`: use persistent SQLite database
    /// - `None`: use in-memory mode (`:memory:`)
    pub mailbox_path: Option<PathBuf>,

    /// Service tags (e.g., "latest", "stable", "v1.0")
    pub tags: Vec<String>,

    /// Script commands
    pub scripts: HashMap<String, String>,

    /// WebRTC configuration
    pub webrtc: WebRtcConfig,

    /// Port for listening to inbound WebSocket connections (direct mode, optional)
    ///
    /// When configured, the node starts a WebSocket server on this port at startup.
    /// Peer nodes can connect directly via `ws://<host-IP>:<port>/` without relaying.
    pub websocket_listen_port: Option<u16>,

    /// WebSocket hostname or IP advertised to the signaling server (direct mode, optional)
    ///
    /// Used together with `websocket_listen_port`. Reported to the signaling server
    /// during registration so that peer nodes know how to connect directly.
    /// Defaults to `"127.0.0.1"` (suitable for local testing only).
    pub websocket_advertised_host: Option<String>,

    /// Observability configuration (logging + tracing)
    pub observability: ObservabilityConfig,

    /// Directory containing the source configuration file (`manifest.toml` or runtime `actr.toml`)
    /// Used for resolving relative paths and finding lock files
    pub config_dir: PathBuf,

    /// Hyper data directory (.hyper), resolved relatively or absolutely from config_dir
    pub hyper_data_dir: PathBuf,

    /// Trust mode: "development" or "production"
    pub trust_mode: String,

    /// Actor execution mode (affects credential acquisition and Hyper cooperation strategy)
    ///
    /// Corresponds to `[system.deployment] mode = "native" | "process" | "wasm"`, defaults to `Native`.
    pub execution_mode: ActrMode,

    /// AIS (Actor Identity Service) HTTP endpoint for credential registration.
    ///
    /// Required in native mode. In process/wasm mode, Hyper handles registration.
    /// Corresponds to `[system.deployment] ais_endpoint = "..."` in runtime `actr.toml`.
    pub ais_endpoint: Option<String>,
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
// Config helper methods
// ============================================================================

impl Config {
    /// Get the package's ActrType (for registration)
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

    /// Get all cross-Realm dependencies
    pub fn cross_realm_dependencies(&self) -> Vec<&Dependency> {
        self.dependencies
            .iter()
            .filter(|d| d.realm.realm_id != self.realm.realm_id)
            .collect()
    }

    /// Get a script command
    pub fn get_script(&self, name: &str) -> Option<&str> {
        self.scripts.get(name).map(|s| s.as_str())
    }

    /// List all script names
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

        // Calculate service fingerprint
        let fingerprint =
            actr_service_compat::Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
                .ok()?;

        // Build Protobuf entries
        let protobufs = self
            .exports
            .iter()
            .map(|export| {
                // Calculate individual file fingerprint
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

        // Get current timestamp
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
    fn test_config_methods() {
        let config = Config {
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
            hyper_data_dir: PathBuf::from("."),
            trust_mode: "development".to_string(),
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
            signaling_url: Url::parse("ws://localhost:8081").unwrap(),
            realm: Realm { realm_id: 1001 },
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
            execution_mode: ActrMode::Native,
            ais_endpoint: None,
        };

        // Test dependency lookup
        assert!(config.get_dependency("user-service").is_some());
        assert!(config.get_dependency("not-exists").is_none());

        // Test cross-Realm dependency
        let cross_realm = config.cross_realm_dependencies();
        assert_eq!(cross_realm.len(), 1);
        assert_eq!(cross_realm[0].alias, "shared-logger");

        // Test fingerprint matching (with service field = strict matching)
        let user_dep = config.get_dependency("user-service").unwrap();
        assert!(user_dep.matches_fingerprint("abc123"));
        assert!(!user_dep.matches_fingerprint("different"));

        // No service field = loose dependency, any fingerprint matches
        let logger_dep = config.get_dependency("shared-logger").unwrap();
        assert!(logger_dep.matches_fingerprint("any-fingerprint"));
        assert!(!logger_dep.requires_exact_fingerprint());
    }
}
