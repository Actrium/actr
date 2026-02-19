use serde::{Deserialize, Serialize};

/// HTTPS 服务绑定配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HttpsBindConfig {
    /// 域名
    ///
    /// 服务绑定的域名，必须与 SSL 证书中的域名匹配。
    pub domain_name: String,

    /// 公网 IP 地址
    ///
    /// 服务对外宣告的 IP 地址，客户端将使用此地址连接。
    pub advertised_ip: String,

    /// 绑定 IP 地址
    ///
    /// 服务实际绑定的网络接口 IP 地址。
    pub ip: String,

    /// 绑定端口
    ///
    /// HTTPS 服务监听的端口号。标准 HTTPS 端口为 443。
    pub port: u16,

    /// SSL 证书文件路径
    ///
    /// PEM 格式的 SSL 证书文件路径。可以是自签名证书或 CA 签发的证书。
    pub cert: String,

    /// SSL 私钥文件路径
    ///
    /// 与证书对应的 PEM 格式私钥文件路径。注意保护私钥文件的安全。
    pub key: String,
}

impl Default for HttpsBindConfig {
    fn default() -> Self {
        Self {
            domain_name: "localhost".to_string(),
            advertised_ip: "127.0.0.1".to_string(),
            ip: "0.0.0.0".to_string(),
            port: 8443,
            cert: "certificates/server.crt".to_string(),
            key: "certificates/server.key".to_string(),
        }
    }
}
