//! Hyper actr runtime configuration structures - direct TOML mapping for actr.toml
//!
//! `ActrRawConfig` maps the flat-section layout of `actr.toml`, which contains
//! deployment, signaling, and observability settings.

use crate::error::Result;
use crate::raw::{
    RawAisEndpointConfig, RawDeploymentConfig, RawDiscoveryConfig, RawObservabilityConfig,
    RawSignalingConfig, RawWebRtcConfig, RawWebSocketConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// Direct mapping of actr.toml (actr runtime configuration, no package info)
///
/// Unlike `RawConfig` which has `[system.signaling]`, `ActrRawConfig` uses
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
pub struct ActrRawConfig {
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

    /// Service capabilities (for signaling load balancing)
    #[serde(default)]
    pub capabilities: Option<RawCapabilitiesConfig>,

    /// Access control list (raw TOML value, parsed later)
    #[serde(default)]
    pub acl: Option<toml::Value>,

    /// Script commands (dev-time only)
    #[serde(default)]
    pub scripts: HashMap<String, String>,
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

fn default_edition() -> u32 {
    1
}

impl ActrRawConfig {
    /// Load actr runtime configuration from file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }
}

impl FromStr for ActrRawConfig {
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
        let config = ActrRawConfig::from_str(toml_content).unwrap();
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
        let config = ActrRawConfig::from_str(toml_content).unwrap();
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
        assert_eq!(
            config.observability.filter_level.as_deref(),
            Some("info")
        );

        let caps = config.capabilities.unwrap();
        assert_eq!(caps.max_concurrent_requests, Some(100));
        assert_eq!(caps.region.as_deref(), Some("cn-beijing"));
        assert_eq!(
            caps.tags.as_ref().and_then(|t| t.get("env")).map(|s| s.as_str()),
            Some("prod")
        );

        assert!(config.acl.is_some());
        assert_eq!(config.scripts.get("dev").map(|s| s.as_str()), Some("cargo run"));
    }

    #[test]
    fn test_parse_empty_actr_config() {
        let toml_content = "edition = 1\n";
        let config = ActrRawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert!(config.signaling.url.is_none());
        assert!(config.capabilities.is_none());
    }
}
