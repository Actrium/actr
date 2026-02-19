use serde::{Deserialize, Serialize};

/// ICE 服务绑定配置
///
/// 用于 STUN 和 TURN 服务的 UDP 网络配置。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IceBindConfig {
    /// 域名
    ///
    /// ICE 服务的域名标识。
    pub domain_name: String,

    /// 绑定 IP 地址
    ///
    /// UDP 服务绑定的网络接口 IP 地址。
    pub ip: String,

    /// 绑定端口
    ///
    /// STUN/TURN 服务监听的 UDP 端口。标准端口为 3478。
    pub port: u16,
}

impl Default for IceBindConfig {
    fn default() -> Self {
        Self {
            domain_name: "localhost".to_string(),
            ip: "0.0.0.0".to_string(),
            port: 3478,
        }
    }
}
