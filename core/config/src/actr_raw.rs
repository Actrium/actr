//! Hyper runtime configuration structures - direct TOML mapping for actr.toml
//!
//! `RuntimeRawConfig` maps the flat-section layout of `actr.toml`, which contains
//! deployment, signaling, and observability settings.

use crate::error::Result;
use crate::raw::{
    RawAisEndpointConfig, RawDeploymentConfig, RawDiscoveryConfig, RawObservabilityConfig,
    RawSignalingConfig, RawStorageConfig, RawWebRtcConfig, RawWebSocketConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// Direct mapping of actr.toml (runtime configuration, no package info)
///
/// Unlike `ManifestRawConfig` which has `[system.signaling]`, `RuntimeRawConfig` uses
/// flat section names: `[signaling]`, `[deployment]`, etc.
///
/// ```toml
/// edition = 1
///
/// [signaling]
/// url = "ws://localhost:8081/signaling/ws"
///
/// [deployment]
/// realm_id = 33554432
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRawConfig {
    /// Config file format version
    #[serde(default = "default_edition")]
    pub edition: u32,

    /// Signaling server connection
    #[serde(default)]
    pub signaling: RawSignalingConfig,

    /// AIS (Actor Identity Service) endpoint
    #[serde(default)]
    pub ais_endpoint: RawAisEndpointConfig,

    /// Deployment parameters (realm, etc.)
    #[serde(default)]
    pub deployment: RawDeploymentConfig,

    /// Service discovery settings
    #[serde(default)]
    pub discovery: RawDiscoveryConfig,

    /// WebRTC transport configuration
    #[serde(default)]
    pub webrtc: RawWebRtcConfig,

    /// WebSocket direct-connect configuration
    #[serde(default)]
    pub websocket: RawWebSocketConfig,

    /// Observability (logging, tracing)
    #[serde(default)]
    pub observability: RawObservabilityConfig,

    /// Storage constraints (hyper dir, mailbox etc.)
    #[serde(default)]
    pub storage: RawStorageConfig,

    /// Service capabilities (for signaling load balancing)
    #[serde(default)]
    pub capabilities: Option<RawCapabilitiesConfig>,

    /// Access control list (raw TOML value, parsed later)
    #[serde(default)]
    pub acl: Option<toml::Value>,

    /// Script commands (dev-time only)
    #[serde(default)]
    pub scripts: HashMap<String, String>,

    /// Path to the workload package (.actr file)
    ///
    /// When specified, the runtime can automatically load the workload package
    /// without requiring explicit path arguments. Supports both absolute and
    /// relative paths (relative to the config file directory).
    ///
    /// Example:
    /// ```toml
    /// [package]
    /// path = "dist/service.actr"
    /// ```
    #[serde(default)]
    pub package: Option<RawPackagePathConfig>,

    /// Web server configuration for `actr run --web`
    ///
    /// When specified, enables serving the actor as a web application.
    ///
    /// Example:
    /// ```toml
    /// [web]
    /// port = 5174
    /// host = "0.0.0.0"
    /// static_dir = "public"
    /// ```
    #[serde(default)]
    pub web: Option<RawWebConfig>,
}

pub type ActrRawConfig = RuntimeRawConfig;

/// Workload package path configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawPackagePathConfig {
    /// Path to the .actr package file (relative to config dir or absolute)
    #[serde(default)]
    pub path: Option<std::path::PathBuf>,
}

/// Service capabilities declaration (reported to signaling for load balancing)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawCapabilitiesConfig {
    /// Maximum concurrent request handling capacity
    #[serde(default)]
    pub max_concurrent_requests: Option<u32>,

    /// Supported version range (e.g., "1.0.0-2.0.0")
    #[serde(default)]
    pub version_range: Option<String>,

    /// Deployment region (e.g., "cn-beijing")
    #[serde(default)]
    pub region: Option<String>,

    /// Custom tags (key-value pairs)
    #[serde(default)]
    pub tags: Option<HashMap<String, String>>,
}

/// Web server configuration for `actr run --web`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawWebConfig {
    /// HTTP server port (default: 8080)
    #[serde(default = "default_web_port")]
    pub port: u16,

    /// HTTP server bind host (default: "0.0.0.0")
    #[serde(default = "default_web_host")]
    pub host: String,

    /// Directory to serve static files from (relative to config dir, default: "public")
    #[serde(default = "default_web_static_dir")]
    pub static_dir: String,

    /// Whether this instance acts as a server (`true`) or client (`false`)
    pub is_server: Option<bool>,

    /// URL path to the .actr package (served from static dir, e.g. "/packages/echo-server.actr")
    pub package_url: Option<String>,

    /// URL path to the shared runtime WASM (e.g. "/packages/actr_runtime_sw_bg.wasm")
    pub runtime_wasm_url: Option<String>,

    /// MFR public key for package verification (Base64-encoded Ed25519 public key)
    pub mfr_pubkey: Option<String>,
}

fn default_web_port() -> u16 {
    8080
}

fn default_web_host() -> String {
    "0.0.0.0".to_string()
}

fn default_web_static_dir() -> String {
    "public".to_string()
}

fn default_edition() -> u32 {
    1
}

impl RuntimeRawConfig {
    /// Load actr runtime configuration from file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }
}

impl FromStr for RuntimeRawConfig {
    type Err = crate::error::ConfigError;

    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_actr_config() {
        let toml_content = r#"
edition = 1

[signaling]
url = "ws://localhost:8081/signaling/ws"

[deployment]
realm_id = 1001
"#;
        let config = RuntimeRawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert_eq!(
            config.signaling.url.as_deref(),
            Some("ws://localhost:8081/signaling/ws")
        );
        assert_eq!(config.deployment.realm_id, Some(1001));
    }

    #[test]
    fn test_parse_full_actr_config() {
        let toml_content = r#"
edition = 1

[signaling]
url = "ws://localhost:8081/signaling/ws"

[ais_endpoint]
url = "http://localhost:8081/ais"

[deployment]
realm_id = 33554432
realm_secret = "rs_test123"

[discovery]
visible = true

[webrtc]
force_relay = false
stun_urls = ["stun:localhost:3478"]
turn_urls = ["turn:localhost:3478"]

[websocket]
listen_port = 9001
advertised_host = "127.0.0.1"

[observability]
filter_level = "info"
tracing_enabled = false

[capabilities]
max_concurrent_requests = 100
version_range = "1.0.0-2.0.0"
region = "cn-beijing"

[capabilities.tags]
env = "prod"
tier = "premium"

[acl]

[[acl.rules]]
permission = "allow"
type = "acme:EchoService:1.0.0"

[scripts]
dev = "cargo run"
test = "cargo test"
"#;
        let config = RuntimeRawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert_eq!(
            config.ais_endpoint.url.as_deref(),
            Some("http://localhost:8081/ais")
        );
        assert_eq!(
            config.deployment.realm_secret.as_deref(),
            Some("rs_test123")
        );
        assert_eq!(config.discovery.visible, Some(true));
        assert!(!config.webrtc.force_relay);
        assert_eq!(config.webrtc.stun_urls.len(), 1);
        assert_eq!(config.websocket.listen_port, Some(9001));
        assert_eq!(config.observability.filter_level.as_deref(), Some("info"));

        let caps = config.capabilities.unwrap();
        assert_eq!(caps.max_concurrent_requests, Some(100));
        assert_eq!(caps.region.as_deref(), Some("cn-beijing"));
        assert_eq!(
            caps.tags
                .as_ref()
                .and_then(|t| t.get("env"))
                .map(|s| s.as_str()),
            Some("prod")
        );

        assert!(config.acl.is_some());
        assert_eq!(
            config.scripts.get("dev").map(|s| s.as_str()),
            Some("cargo run")
        );
    }

    #[test]
    fn test_parse_empty_actr_config() {
        let toml_content = "edition = 1\n";
        let config = ActrRawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert!(config.signaling.url.is_none());
        assert!(config.capabilities.is_none());
    }

    #[test]
    fn test_parse_actr_config_with_package_path() {
        let toml_content = r#"
edition = 1

[signaling]
url = "ws://localhost:8081/signaling/ws"

[deployment]
realm_id = 1001

[package]
path = "dist/service.actr"
"#;
        let config = RuntimeRawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert!(config.package.is_some());
        let package = config.package.unwrap();
        assert_eq!(
            package.path.as_ref().map(|p| p.to_str().unwrap()),
            Some("dist/service.actr")
        );
    }
}
