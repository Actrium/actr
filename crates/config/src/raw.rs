//! Raw configuration structures - direct TOML mapping

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Actr.toml 的直接映射（无任何处理）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawConfig {
    /// 配置文件格式版本（决定使用哪个 Parser）
    #[serde(default = "default_edition")]
    pub edition: u32,

    /// 继承的父配置文件路径
    #[serde(default)]
    pub inherit: Option<PathBuf>,

    /// 包信息
    pub package: RawPackageConfig,

    /// 导出的 proto 文件列表
    #[serde(default)]
    pub exports: Vec<PathBuf>,

    /// 服务依赖
    #[serde(default)]
    pub dependencies: HashMap<String, RawDependency>,

    /// 系统配置
    #[serde(default)]
    pub system: RawSystemConfig,

    /// 访问控制列表（原始 TOML 值，稍后解析）
    #[serde(default)]
    pub acl: Option<toml::Value>,

    /// 脚本命令
    #[serde(default)]
    pub scripts: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawPackageConfig {
    pub name: String,
    pub manufacturer: String,
    #[serde(rename = "type")]
    pub type_name: String,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub authors: Option<Vec<String>>,

    #[serde(default)]
    pub license: Option<String>,

    /// Service tags (e.g., ["latest", "stable", "v1.0"])
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawDependency {
    /// 带指纹的依赖配置（必须先匹配，因为它有 required 字段）
    WithFingerprint {
        #[serde(default)]
        realm: Option<u32>,

        #[serde(default)]
        actr_type: Option<String>,

        fingerprint: String,
    },

    /// 空依赖声明：{}（由 actr install 填充）
    Empty {},
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawSystemConfig {
    #[serde(default)]
    pub signaling: RawSignalingConfig,

    #[serde(default)]
    pub deployment: RawDeploymentConfig,

    #[serde(default)]
    pub discovery: RawDiscoveryConfig,

    #[serde(default)]
    pub storage: RawStorageConfig,

    #[serde(default)]
    pub webrtc: RawWebRtcConfig,
    pub observability: RawObservabilityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawSignalingConfig {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawDeploymentConfig {
    #[serde(default)]
    pub realm: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawDiscoveryConfig {
    #[serde(default)]
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawStorageConfig {
    #[serde(default)]
    pub mailbox_path: Option<PathBuf>,
}

/// WebRTC 配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawWebRtcConfig {
    /// STUN 服务器 URL 列表 (例如 ["stun:localhost:3478"])
    #[serde(default)]
    pub stun_urls: Vec<String>,

    /// TURN 服务器 URL 列表 (例如 ["turn:localhost:3478"])
    #[serde(default)]
    pub turn_urls: Vec<String>,

    /// 是否强制使用 TURN 中继 (默认 false)
    #[serde(default)]
    pub force_relay: bool,
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

impl RawConfig {
    /// 从文件加载原始配置
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }

    /// 保存到文件
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

impl FromStr for RawConfig {
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

        let config = RawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert_eq!(config.package.name, "test-service");
        assert_eq!(config.exports.len(), 1);
        assert!(config.dependencies.contains_key("user-service"));
    }

    #[test]
    fn test_parse_complex_dependency() {
        let toml_content = r#"
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

        let config = RawConfig::from_str(toml_content).unwrap();
        let dep = config.dependencies.get("shared").unwrap();

        match dep {
            RawDependency::WithFingerprint {
                realm,
                actr_type,
                fingerprint,
            } => {
                assert_eq!(*realm, Some(9999));
                assert_eq!(actr_type.as_ref().unwrap(), "logging-service");
                assert_eq!(fingerprint, "service_semantic:abc123...");
            }
            _ => panic!("Expected fingerprint dependency"),
        }
    }
}
