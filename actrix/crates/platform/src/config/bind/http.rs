use serde::{Deserialize, Serialize};

/// HTTP/HTTPS 服务绑定配置
///
/// 统一的 HTTP 绑定配置。配置了 `cert` 和 `key` 即启用 TLS (HTTPS)，
/// 否则为纯 HTTP 模式。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HttpBindConfig {
    /// 域名
    ///
    /// 服务绑定的域名，用于生成正确的 URL 和证书验证。
    pub domain_name: String,

    /// 公网 IP 地址
    ///
    /// 服务对外宣告的 IP 地址，客户端将使用此地址连接。
    /// 在 NAT 环境中，这通常是路由器的公网 IP。
    pub advertised_ip: String,

    /// 绑定 IP 地址
    ///
    /// 服务实际绑定的网络接口 IP 地址。
    /// 默认使用 "::" 以在支持双栈的系统上同时覆盖 IPv4/IPv6。
    pub ip: String,

    /// 绑定端口
    ///
    /// 服务监听的端口号。
    pub port: u16,

    /// 公网宣告端口（可选）
    ///
    /// 对外宣告的端口号。在 NAT/反向代理环境中可能与绑定端口不同。
    /// 默认与 `port` 相同。
    #[serde(default)]
    pub advertised_port: Option<u16>,

    /// TLS 证书文件路径（可选）
    ///
    /// PEM 格式的 TLS 证书文件路径。配置此字段和 `key` 即启用 HTTPS。
    #[serde(default)]
    pub cert: Option<String>,

    /// TLS 私钥文件路径（可选）
    ///
    /// 与证书对应的 PEM 格式私钥文件路径。
    #[serde(default)]
    pub key: Option<String>,
}

impl HttpBindConfig {
    /// 是否启用 TLS
    ///
    /// 当 `cert` 和 `key` 都配置了非空值时返回 true。
    pub fn is_tls(&self) -> bool {
        self.cert.as_ref().is_some_and(|c| !c.is_empty())
            && self.key.as_ref().is_some_and(|k| !k.is_empty())
    }

    /// 返回有效的宣告端口
    ///
    /// 优先使用 `advertised_port`，未配置则回退到 `port`。
    pub fn effective_advertised_port(&self) -> u16 {
        self.advertised_port.unwrap_or(self.port)
    }

    /// 返回协议方案字符串
    pub fn scheme(&self) -> &'static str {
        if self.is_tls() { "https" } else { "http" }
    }

    /// 返回 WebSocket 协议方案字符串
    pub fn ws_scheme(&self) -> &'static str {
        if self.is_tls() { "wss" } else { "ws" }
    }
}

impl Default for HttpBindConfig {
    fn default() -> Self {
        Self {
            domain_name: "localhost".to_string(),
            advertised_ip: "127.0.0.1".to_string(),
            ip: "::".to_string(),
            port: 8080,
            advertised_port: None,
            cert: None,
            key: None,
        }
    }
}
