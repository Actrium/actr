//! Configuration for supervit client

use crate::error::{Result, SupervitError};
use serde::{Deserialize, Serialize};

/// Supervit 客户端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervitConfig {
    /// 节点唯一标识符
    pub node_id: String,

    /// 节点可读名称（可选，未设置时使用 node_id）
    #[serde(default)]
    pub name: Option<String>,

    /// This field is skipped by serde during serialization and deserialization and must be set via code logic.
    #[serde(skip, default)]
    pub location_tag: String,

    /// Supervisor gRPC 服务器地址
    /// 格式: http://hostname:port 或 https://hostname:port
    /// 示例: "http://supervisor.example.com:50051"
    pub endpoint: String,

    /// Supervisord gRPC advertised address (for Supervisor callback)
    ///
    /// This is the address that Supervisor will use to connect back to this node.
    /// Format: "ip:port" (e.g., "203.0.113.10:50055")
    /// This value is typically passed from SupervisorConfig.advertised_addr().
    #[serde(default = "default_agent_addr")]
    pub agent_addr: String,

    /// 连接超时（秒）
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,

    /// 状态上报间隔（秒）
    #[serde(default = "default_status_interval")]
    pub status_report_interval_secs: u64,

    /// 健康检查间隔（秒）
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_secs: u64,

    /// 是否启用 TLS
    #[serde(default)]
    pub enable_tls: bool,

    /// TLS 域名（用于证书验证）
    pub tls_domain: Option<String>,

    /// 客户端证书路径（用于 mTLS）
    pub client_cert: Option<String>,

    /// 客户端私钥路径（用于 mTLS）
    pub client_key: Option<String>,

    /// CA 证书路径（用于验证服务端证书）
    pub ca_cert: Option<String>,

    /// 共享密钥（hex 编码，用于 HMAC 签名）
    ///
    /// 必须至少 32 字节（64 个 hex 字符）
    pub shared_secret: Option<String>,

    /// 最大允许的时钟偏差（秒）
    #[serde(default = "default_max_clock_skew")]
    pub max_clock_skew_secs: u64,

    /// 可选的自由格式位置描述（与 location_tag 区分）
    #[serde(default)]
    pub location: Option<String>,

    /// 可选的服务标签（应用于全部服务）
    #[serde(default)]
    pub service_tags: Vec<String>,
}

fn default_connect_timeout() -> u64 {
    30
}

fn default_status_interval() -> u64 {
    60
}

fn default_health_check_interval() -> u64 {
    30
}

fn default_max_clock_skew() -> u64 {
    300 // 5 分钟
}

fn default_agent_addr() -> String {
    "0.0.0.0:50055".to_string()
}

impl Default for SupervitConfig {
    fn default() -> Self {
        Self {
            node_id: String::new(),
            name: None,
            location_tag: String::new(),
            endpoint: "http://localhost:50051".to_string(),
            agent_addr: default_agent_addr(),
            connect_timeout_secs: default_connect_timeout(),
            status_report_interval_secs: default_status_interval(),
            health_check_interval_secs: default_health_check_interval(),
            enable_tls: false,
            tls_domain: None,
            client_cert: None,
            client_key: None,
            ca_cert: None,
            shared_secret: None,
            max_clock_skew_secs: default_max_clock_skew(),
            location: None,
            service_tags: Vec::new(),
        }
    }
}

impl SupervitConfig {
    /// 验证配置有效性
    pub fn validate(&self) -> Result<()> {
        if self.node_id.is_empty() {
            return Err(SupervitError::Config("node_id cannot be empty".to_string()));
        }

        if self.endpoint.is_empty() {
            return Err(SupervitError::Config(
                "endpoint cannot be empty".to_string(),
            ));
        }

        if self.agent_addr.is_empty() {
            return Err(SupervitError::Config(
                "agent_addr cannot be empty".to_string(),
            ));
        }

        if !self.endpoint.starts_with("http://") && !self.endpoint.starts_with("https://") {
            return Err(SupervitError::Config(
                "endpoint must start with http:// or https://".to_string(),
            ));
        }

        if self.enable_tls && self.tls_domain.is_none() {
            return Err(SupervitError::Config(
                "tls_domain is required when enable_tls is true".to_string(),
            ));
        }

        // 验证 mTLS 配置的完整性
        if self.client_cert.is_some() || self.client_key.is_some() {
            if self.client_cert.is_none() || self.client_key.is_none() {
                return Err(SupervitError::Config(
                    "Both client_cert and client_key must be provided for mTLS".to_string(),
                ));
            }
            if !self.enable_tls {
                return Err(SupervitError::Config(
                    "enable_tls must be true when using mTLS".to_string(),
                ));
            }
        }

        // 验证 shared_secret 长度
        if let Some(ref secret) = self.shared_secret {
            if secret.len() < 64 {
                return Err(SupervitError::Config(
                    "shared_secret must be at least 64 hex characters (32 bytes)".to_string(),
                ));
            }
            // 验证是否为有效的 hex 字符串
            if hex::decode(secret).is_err() {
                return Err(SupervitError::Config(
                    "shared_secret must be a valid hex string".to_string(),
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SupervitConfig::default();
        assert_eq!(config.endpoint, "http://localhost:50051");
        assert_eq!(config.connect_timeout_secs, 30);
    }

    #[test]
    fn test_validate_empty_node_id() {
        let config = SupervitConfig {
            node_id: String::new(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_url() {
        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "invalid-url".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_valid_config() {
        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }
}
