//! Signaling 服务配置

use crate::config::ks::KsClientConfig;
use serde::{Deserialize, Serialize};

/// Signaling 服务配置
///
/// Service enable/disable is controlled by the bitmask in ActrixConfig.enable.
/// The ENABLE_SIGNALING bit (bit 0) must be set to enable this service.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SignalingConfig {
    /// Signaling 服务器配置
    #[serde(default)]
    pub server: SignalingServerConfig,

    /// Signaling 的依赖服务配置
    #[serde(default)]
    pub dependencies: SignalingDependencies,
}

/// Signaling 服务器配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SignalingServerConfig {
    /// WebSocket 路径
    pub ws_path: String,

    /// 速率限制配置
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

/// 速率限制配置
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RateLimitConfig {
    /// 连接速率限制配置
    #[serde(default)]
    pub connection: ConnectionRateLimit,

    /// 消息速率限制配置
    #[serde(default)]
    pub message: MessageRateLimit,
}

/// 连接速率限制配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConnectionRateLimit {
    /// 是否启用连接速率限制
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// 每分钟允许的新连接数
    #[serde(default = "default_connections_per_minute")]
    pub per_minute: u32,

    /// 突发允许的连接数
    #[serde(default = "default_connection_burst")]
    pub burst_size: u32,

    /// 每个 IP 的最大并发连接数
    #[serde(default = "default_max_concurrent_connections")]
    pub max_concurrent_per_ip: u32,
}

/// 消息速率限制配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageRateLimit {
    /// 是否启用消息速率限制
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// 每秒允许的消息数
    #[serde(default = "default_messages_per_second")]
    pub per_second: u32,

    /// 突发允许的消息数
    #[serde(default = "default_message_burst")]
    pub burst_size: u32,
}

// 默认值函数
fn default_true() -> bool {
    true
}

fn default_connections_per_minute() -> u32 {
    5
}

fn default_connection_burst() -> u32 {
    10
}

fn default_max_concurrent_connections() -> u32 {
    100
}

fn default_messages_per_second() -> u32 {
    10
}

fn default_message_burst() -> u32 {
    50
}

/// Signaling 依赖的外部服务
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SignalingDependencies {
    /// KS 客户端配置（可选，如果需要加密）
    ///
    /// 如果未配置但需要 KS，会自动查找本地 KS 服务：
    /// - 如果 KS 服务已启用（ENABLE_KS 位已设置），使用 localhost:KS_PORT
    /// - 否则返回 None（Signaling 可以不依赖 KS）
    #[serde(default)]
    pub ks: Option<KsClientConfig>,

    /// AIS 客户端配置（可选，用于 Credential 刷新）
    ///
    /// 如果未配置但需要 AIS，会自动查找本地 AIS 服务：
    /// - 如果 AIS 服务已启用（ENABLE_AIS 位已设置），使用 localhost:AIS_PORT
    /// - 否则返回 None
    #[serde(default)]
    pub ais: Option<AisClientConfig>,
}

/// AIS 客户端配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AisClientConfig {
    /// AIS 服务端点 URL
    pub endpoint: String,
    /// 请求超时时间（秒）
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_timeout() -> u64 {
    30
}

impl Default for SignalingServerConfig {
    fn default() -> Self {
        Self {
            ws_path: "/signaling".to_string(),
            rate_limit: RateLimitConfig::default(),
        }
    }
}

impl Default for ConnectionRateLimit {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            per_minute: default_connections_per_minute(),
            burst_size: default_connection_burst(),
            max_concurrent_per_ip: default_max_concurrent_connections(),
        }
    }
}

impl Default for MessageRateLimit {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            per_second: default_messages_per_second(),
            burst_size: default_message_burst(),
        }
    }
}

impl SignalingConfig {
    /// 获取 KS 客户端配置
    ///
    /// 支持智能默认：
    /// 1. 如果显式配置了 dependencies.ks，直接返回
    /// 2. 如果本地启用了 KS 服务，返回指向本地 KS 的配置
    /// 3. 否则返回 None（Signaling 可以不依赖 KS）
    pub fn get_ks_client_config(
        &self,
        global_config: &super::ActrixConfig,
    ) -> Option<KsClientConfig> {
        // 优先使用显式配置
        if let Some(ref ks_config) = self.dependencies.ks {
            return Some(ks_config.clone());
        }

        // 回退：检查是否启用了本地 KS 服务
        if global_config.is_ks_enabled() && global_config.services.ks.is_some() {
            // 自动生成指向本地 KS 的客户端配置
            // gRPC 使用独立端口 50052（HTTP router 使用 8443/8080）
            let grpc_port = 50052;
            let grpc_protocol = "http"; // 默认不启用 TLS，可通过配置开启

            return Some(KsClientConfig {
                endpoint: format!("{grpc_protocol}://127.0.0.1:{grpc_port}"),
                timeout_seconds: 30,
                enable_tls: false,
                tls_domain: None,
                ca_cert: None,
                client_cert: None,
                client_key: None,
            });
        }

        None
    }

    /// 获取 AIS 客户端配置
    ///
    /// 支持智能默认：
    /// 1. 如果显式配置了 dependencies.ais，直接返回
    /// 2. 如果本地启用了 AIS 服务，返回指向本地 AIS 的配置
    /// 3. 否则返回 None
    pub fn get_ais_client_config(
        &self,
        global_config: &super::ActrixConfig,
    ) -> Option<AisClientConfig> {
        // 优先使用显式配置
        if let Some(ref ais_config) = self.dependencies.ais {
            return Some(ais_config.clone());
        }

        // 回退：检查是否启用了本地 AIS 服务
        if global_config.is_ais_enabled() && global_config.services.ais.is_some() {
            // 自动生成指向本地 AIS 的客户端配置
            // AIS 作为 HTTP router service 共享同一个 HTTP/HTTPS 端口
            let port = global_config
                .bind
                .https
                .as_ref()
                .map(|h| h.port)
                .or_else(|| global_config.bind.http.as_ref().map(|h| h.port))
                .unwrap_or(8080);

            let protocol = if global_config.bind.https.is_some() {
                "https"
            } else {
                "http"
            };

            return Some(AisClientConfig {
                endpoint: format!("{protocol}://127.0.0.1:{port}"),
                timeout_seconds: 30,
            });
        }

        None
    }
}
