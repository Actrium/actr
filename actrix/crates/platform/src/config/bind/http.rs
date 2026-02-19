use serde::{Deserialize, Serialize};

/// HTTP 服务绑定配置
#[derive(Debug, Serialize, Deserialize, Clone)]
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
    /// 通常使用 "0.0.0.0" 监听所有接口。
    pub ip: String,

    /// 绑定端口
    ///
    /// HTTP 服务监听的端口号。标准 HTTP 端口为 80。
    pub port: u16,
}

impl Default for HttpBindConfig {
    fn default() -> Self {
        Self {
            domain_name: "localhost".to_string(),
            advertised_ip: "127.0.0.1".to_string(),
            ip: "0.0.0.0".to_string(),
            port: 8080,
        }
    }
}
