//! AIS (Actor Identity Service) 配置

use crate::config::signer::SignerClientConfig;
use serde::{Deserialize, Serialize};

/// AIS 服务配置
///
/// Service enable/disable is controlled by the bitmask in ActrixConfig.enable.
/// The ENABLE_AIS bit (bit 3) must be set to enable this service.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AisConfig {
    /// AIS 服务器配置
    #[serde(default)]
    pub server: AisServerConfig,

    /// AIS 的依赖服务配置
    #[serde(default)]
    pub dependencies: AisDependencies,
}

/// AIS 服务器配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AisServerConfig {
    /// Signaling Server 心跳间隔（秒）
    ///
    /// 在 RegisterResponse 中返回，指导客户端连接 Signaling Server 后的心跳频率
    #[serde(default = "default_signaling_heartbeat_interval_secs")]
    pub signaling_heartbeat_interval_secs: u32,

    /// Token 有效期（秒）
    ///
    /// 生成的 AIdCredential 的过期时间
    #[serde(default = "default_token_ttl_secs")]
    pub token_ttl_secs: u64,

    /// Renewal token 有效期（秒）
    ///
    /// 初始签发 renewal token 时的 TTL。到达 rotation window 时
    /// 确定性派生 successor token，旧 token 在自身 expiry 前持续有效。
    #[serde(default = "default_renewal_token_ttl_secs")]
    pub renewal_token_ttl_secs: u64,

    /// Renewal rotation window（秒）
    ///
    /// 当 renewal token 剩余有效期小于此值时进入 rotation window，
    /// 开始确定性派生 successor token。
    #[serde(default = "default_renewal_rotation_window_secs")]
    pub renewal_rotation_window_secs: u64,

    /// Renewal token secret（base64 编码，解码后至少 32 字节）
    ///
    /// 用于确定性派生 successor token（HMAC-SHA256）。
    /// 必须在同一 actrix 集群内一致。不得使用 `actrix_shared_key` 作为回退。
    #[serde(default)]
    pub renewal_token_secret: String,
}

/// AIS 依赖的外部服务
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AisDependencies {
    /// Signer 客户端配置
    ///
    /// 如果未配置，AIS 会自动查找本地 Signer 服务：
    /// - 如果 Signer 服务已启用（ENABLE_SIGNER 位已设置），使用 localhost:SIGNER_PORT
    /// - 否则返回配置错误
    #[serde(default)]
    pub signer: Option<SignerClientConfig>,
}

impl Default for AisServerConfig {
    fn default() -> Self {
        Self {
            signaling_heartbeat_interval_secs: default_signaling_heartbeat_interval_secs(),
            token_ttl_secs: default_token_ttl_secs(),
            renewal_token_ttl_secs: default_renewal_token_ttl_secs(),
            renewal_rotation_window_secs: default_renewal_rotation_window_secs(),
            renewal_token_secret: String::new(),
        }
    }
}

/// 默认 Signaling Server 心跳间隔：30 秒
fn default_signaling_heartbeat_interval_secs() -> u32 {
    30
}

/// 默认 Token 有效期：1 小时（3600 秒）
fn default_token_ttl_secs() -> u64 {
    3600
}

/// 默认 Renewal Token 有效期：24 小时（86400 秒）
fn default_renewal_token_ttl_secs() -> u64 {
    86400
}

/// 默认 Renewal Rotation Window：6 小时（21600 秒）
fn default_renewal_rotation_window_secs() -> u64 {
    21600
}

impl AisConfig {
    /// 获取 Signer 客户端配置
    ///
    /// 支持智能默认：
    /// 1. 如果显式配置了 dependencies.signer，直接返回
    /// 2. 如果本地启用了 Signer 服务，返回指向本地 KS 的配置
    /// 3. 否则返回 None
    pub fn get_signer_client_config(
        &self,
        global_config: &super::ActrixConfig,
    ) -> Option<SignerClientConfig> {
        // 优先使用显式配置
        if let Some(ref signer_config) = self.dependencies.signer {
            return Some(signer_config.clone());
        }

        // 回退：检查是否启用了本地 Signer 服务
        if global_config.is_signer_enabled() && global_config.services.signer.is_some() {
            // 自动生成指向本地 Signer 的客户端配置
            // Signer gRPC 复用实例主 HTTP/HTTPS 端口
            let http_cfg = global_config.bind.http.as_ref();
            let port = http_cfg.map(|h| h.port).unwrap_or(8080);
            let use_tls = http_cfg.is_some_and(|h| h.is_tls());
            let protocol = if use_tls { "https" } else { "http" };
            let tls_domain = if use_tls {
                http_cfg.map(|h| h.domain_name.clone())
            } else {
                None
            };

            return Some(SignerClientConfig {
                endpoint: format!("{protocol}://127.0.0.1:{port}"),
                timeout_seconds: 30,
                enable_tls: use_tls,
                tls_domain,
                ca_cert: None,
                client_cert: None,
                client_key: None,
            });
        }

        None
    }
}
