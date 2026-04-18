//! Raw configuration structures - direct TOML mapping

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Direct mapping of manifest.toml (no processing applied)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestRawConfig {
    /// Config file format version (determines which Parser to use)
    #[serde(default = "default_edition")]
    pub edition: u32,

    /// Inherited parent config file path
    #[serde(default)]
    pub inherit: Option<PathBuf>,

    /// Directory containing the lock file
    #[serde(default)]
    pub config_dir: Option<PathBuf>,

    /// Package info
    pub package: RawPackageConfig,

    /// Exported proto file list
    #[serde(default)]
    pub exports: Vec<PathBuf>,

    /// Service dependencies
    #[serde(default)]
    pub dependencies: HashMap<String, RawDependency>,

    /// Access control list (raw TOML value, parsed later)
    #[serde(default)]
    pub acl: Option<toml::Value>,

    /// Script commands
    #[serde(default)]
    pub scripts: HashMap<String, String>,

    /// Final packaged binary configuration
    #[serde(default)]
    pub binary: Option<RawBinaryConfig>,

    /// Source build configuration
    #[serde(default)]
    pub build: Option<RawBuildConfig>,
}

pub type RawConfig = ManifestRawConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawPackageConfig {
    /// Package name (also used as the actor type name)
    pub name: String,

    /// Manufacturer identifier (e.g., "acme")
    pub manufacturer: String,

    /// Semantic version (e.g., "1.0.0"). Defaults to empty string if not specified.
    #[serde(default)]
    pub version: String,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub authors: Option<Vec<String>>,

    #[serde(default)]
    pub license: Option<String>,

    /// Service tags (e.g., ["latest", "stable", "v1.0"])
    #[serde(default)]
    pub tags: Vec<String>,

    /// Signature algorithm (default: "ed25519")
    #[serde(default)]
    pub signature_algorithm: Option<String>,

    /// Exported proto file paths (new location, preferred over top-level `exports`)
    #[serde(default)]
    pub exports: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawBinaryConfig {
    pub path: PathBuf,

    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawBuildConfig {
    #[serde(default)]
    pub tool: Option<String>,

    #[serde(default)]
    pub manifest_path: Option<PathBuf>,

    #[serde(default)]
    pub artifact: Option<String>,

    #[serde(default)]
    pub target: Option<String>,

    #[serde(default)]
    pub profile: Option<String>,

    #[serde(default)]
    pub features: Vec<String>,

    #[serde(default)]
    pub no_default_features: bool,

    #[serde(default)]
    pub post_build: Vec<String>,
}

impl RawPackageConfig {
    pub fn into_package_info(self) -> Result<crate::config::PackageInfo> {
        Ok(crate::config::PackageInfo {
            name: self.name.clone(),
            actr_type: actr_protocol::ActrType {
                manufacturer: self.manufacturer.clone(),
                name: self.name,
                version: self.version,
            },
            description: self.description,
            authors: self.authors.unwrap_or_default(),
            license: self.license,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawDependency {
    /// Dependency with specified ActrType (matched first since it has the required `actr_type` field)
    ///
    /// Example:
    /// ```toml
    /// [dependencies]
    /// echo = { actr_type = "acme:echo-service:1.0.0", service = "EchoService:abc1f3d" }
    /// ```
    Specified {
        /// Full ActrType string: "manufacturer:name:version"
        #[serde(rename = "actr_type")]
        actr_type: String,

        /// Optional strict service reference: "ServiceName:fingerprint".
        /// When present, enables exact proto fingerprint matching.
        #[serde(default)]
        service: Option<String>,

        /// Optional cross-realm override. Defaults to self realm.
        #[serde(default)]
        realm: Option<u32>,
    },

    /// Empty dependency declaration: {} (populated by actr deps install)
    Empty {},
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawSystemConfig {
    #[serde(default)]
    pub signaling: RawSignalingConfig,

    #[serde(default)]
    pub ais_endpoint: RawAisEndpointConfig,

    #[serde(default)]
    pub deployment: RawDeploymentConfig,

    #[serde(default)]
    pub discovery: RawDiscoveryConfig,

    #[serde(default)]
    pub storage: RawStorageConfig,

    #[serde(default)]
    pub webrtc: RawWebRtcConfig,
    #[serde(default)]
    pub websocket: RawWebSocketConfig,
    #[serde(default)]
    pub observability: RawObservabilityConfig,
}

/// WebSocket data transport configuration
///
/// Configuration example (actr.toml):
/// ```toml
/// [system.websocket]
/// listen_port = 9001
/// advertised_host = "192.168.1.10"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawWebSocketConfig {
    /// Port for listening to inbound WebSocket connections (for direct mode)
    ///
    /// When configured, the node starts a WebSocket server on this port,
    /// accepting direct connections from peer nodes.
    /// When not configured, the node does not listen on any port (relay mode only).
    #[serde(default)]
    pub listen_port: Option<u16>,

    /// Externally advertised WebSocket hostname or IP (for signaling registration)
    ///
    /// When a node has `listen_port` configured, the signaling server needs an address
    /// reachable by peer nodes. This field specifies the hostname or IP registered with
    /// the signaling server, e.g., `"192.168.1.10"` or `"mynode.example.com"`.
    /// Defaults to `"127.0.0.1"` if not configured (suitable for local testing only).
    #[serde(default)]
    pub advertised_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawSignalingConfig {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawAisEndpointConfig {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawDeploymentConfig {
    #[serde(default)]
    pub realm_id: Option<u32>,

    /// Realm secret for AIS registration authentication
    #[serde(default)]
    pub realm_secret: Option<String>,

    /// AIS (Actor Identity Service) HTTP endpoint, e.g. `"http://ais.example.com:8080"`.
    #[serde(default)]
    pub ais_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawDiscoveryConfig {
    #[serde(default)]
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawStorageConfig {
    #[serde(default)]
    pub mailbox_path: Option<PathBuf>,
}

/// WebRTC configuration
///
/// Without port range configured, uses default mode (random ports).
/// With port_range_start/end configured, enables fixed port mode.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawWebRtcConfig {
    /// STUN server URL list (e.g., ["stun:localhost:3478"])
    #[serde(default)]
    pub stun_urls: Vec<String>,

    /// TURN server URL list (e.g., ["turn:localhost:3478"])
    #[serde(default)]
    pub turn_urls: Vec<String>,

    /// Whether to force TURN relay (default false)
    #[serde(default)]
    pub force_relay: bool,

    /// ICE host candidate acceptance min wait (ms)
    #[serde(default)]
    pub ice_host_acceptance_min_wait: Option<u64>,

    /// ICE srflx candidate acceptance min wait (ms)
    #[serde(default)]
    pub ice_srflx_acceptance_min_wait: Option<u64>,

    /// ICE prflx candidate acceptance min wait (ms)
    #[serde(default)]
    pub ice_prflx_acceptance_min_wait: Option<u64>,

    /// ICE relay candidate acceptance min wait (ms)
    #[serde(default)]
    pub ice_relay_acceptance_min_wait: Option<u64>,

    /// UDP port range start (optional, enables fixed port mode when configured)
    #[serde(default)]
    pub port_range_start: Option<u16>,

    /// UDP port range end (optional, enables fixed port mode when configured)
    #[serde(default)]
    pub port_range_end: Option<u16>,

    /// NAT 1:1 public IP mapping (optional)
    #[serde(default)]
    pub public_ips: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawObservabilityConfig {
    /// Filter level (e.g., "info", "debug", "warn", "info,webrtc=debug").
    /// Used when RUST_LOG environment variable is not set. Default: "info".
    #[serde(default)]
    pub filter_level: Option<String>,

    #[serde(default)]
    pub tracing_enabled: Option<bool>,

    /// OTLP/Jaeger gRPC endpoint. Default: http://localhost:4317
    #[serde(default)]
    pub tracing_endpoint: Option<String>,

    /// Service name reported to the tracing backend. Default: package.name
    #[serde(default)]
    pub tracing_service_name: Option<String>,
}

fn default_edition() -> u32 {
    1
}

impl ManifestRawConfig {
    /// Load raw configuration from file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }

    /// Save to file
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

impl FromStr for ManifestRawConfig {
    type Err = crate::error::ConfigError;

    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let config = ManifestRawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert_eq!(config.package.name, "test-service");
        assert_eq!(config.exports.len(), 1);
        assert!(config.dependencies.contains_key("user-service"));
    }

    #[test]
    fn test_parse_dependency_with_empty_attributes() {
        let toml_content = r#"
[package]
name = "test"
manufacturer = "acme"
[dependencies]
user-service = {}
"#;
        let config = ManifestRawConfig::from_str(toml_content).unwrap();
        let dep = config.dependencies.get("user-service").unwrap();
        assert!(matches!(dep, RawDependency::Empty {}));
    }

    #[test]
    fn test_parse_dependency_specified() {
        let toml_content = r#"
[package]
name = "test"
manufacturer = "acme"
[dependencies]
shared = { actr_type = "acme:logging-service:1.0.0", service = "LoggingService:abc123", realm = 9999 }
"#;
        let config = ManifestRawConfig::from_str(toml_content).unwrap();
        let dep = config.dependencies.get("shared").unwrap();
        if let RawDependency::Specified {
            actr_type,
            service,
            realm,
        } = dep
        {
            assert_eq!(actr_type, "acme:logging-service:1.0.0");
            assert_eq!(service.as_deref(), Some("LoggingService:abc123"));
            assert_eq!(*realm, Some(9999));
        } else {
            panic!("Expected Specified");
        }
    }

    #[test]
    fn test_parse_dependency_specified_no_service() {
        let toml_content = r#"
[package]
name = "test"
manufacturer = "acme"
[dependencies]
shared = { actr_type = "acme:logging-service:1.0.0" }
"#;
        let config = ManifestRawConfig::from_str(toml_content).unwrap();
        let dep = config.dependencies.get("shared").unwrap();
        if let RawDependency::Specified { service, .. } = dep {
            assert!(service.is_none());
        } else {
            panic!("Expected Specified");
        }
    }
}
