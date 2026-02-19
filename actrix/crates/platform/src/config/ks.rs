//! Key Server (KS) 配置
//!
//! KS 服务用于生成和管理加密密钥，为其他服务提供密钥生成和公钥查询功能。

use serde::{Deserialize, Serialize};

/// KS 服务器配置
///
/// 配置 KS 服务的服务器端参数
/// 注意：只存储 key_id 和 public_key，不存储 secret_key。
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct KsServerConfig {}

/// KS 客户端配置
///
/// 其他服务作为客户端连接 KS 服务时使用的配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KsClientConfig {
    /// KS 服务地址
    ///
    /// gRPC endpoint，例如: "http://127.0.0.1:50052" 或 "https://ks.example.com:50052"
    pub endpoint: String,

    /// 请求超时时间（秒）
    ///
    /// 连接 KS 服务的超时时间
    pub timeout_seconds: u64,

    /// 是否启用 TLS
    ///
    /// 默认为 false（使用 HTTP）。设为 true 时使用 HTTPS/gRPC over TLS。
    #[serde(default)]
    pub enable_tls: bool,

    /// TLS 域名（启用 TLS 时必需）
    ///
    /// 用于 TLS 证书验证的域名
    pub tls_domain: Option<String>,

    /// CA 证书路径（用于验证服务端证书）
    ///
    /// 用于验证 KS 服务端证书的 CA 证书文件路径
    pub ca_cert: Option<String>,

    /// 客户端证书路径（mTLS）
    ///
    /// 用于双向 TLS 认证的客户端证书文件路径
    pub client_cert: Option<String>,

    /// 客户端私钥路径（mTLS）
    ///
    /// 用于双向 TLS 认证的客户端私钥文件路径
    pub client_key: Option<String>,
}

/// KS 配置（包含服务器和客户端配置）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KsConfig {
    /// KS 服务器配置（当本实例作为 KS 服务器时使用）
    pub server: Option<KsServerConfig>,

    /// KS 客户端配置（当需要连接其他 KS 服务时使用）
    pub client: Option<KsClientConfig>,
}

impl Default for KsClientConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:50052".to_string(), // gRPC 默认端口
            timeout_seconds: 30,
            enable_tls: false,
            tls_domain: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
        }
    }
}

impl Default for KsConfig {
    fn default() -> Self {
        Self {
            server: Some(KsServerConfig::default()),
            client: Some(KsClientConfig::default()),
        }
    }
}
