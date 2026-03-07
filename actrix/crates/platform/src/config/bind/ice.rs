use serde::{Deserialize, Serialize};

/// ICE 服务绑定配置
///
/// 用于 STUN 和 TURN 服务的 UDP 网络配置。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct IceBindConfig {
    /// 绑定 IP 地址
    ///
    /// UDP 服务绑定的网络接口 IP 地址。
    pub ip: String,

    /// 绑定端口
    ///
    /// STUN/TURN 服务监听的 UDP 端口。标准端口为 3478。
    pub port: u16,

    /// 公网 IP 地址
    ///
    /// ICE 服务对外宣告的公网 IP 地址。客户端通过此地址进行 STUN/TURN 通信。
    /// 必须是真实可路由的公网 IP，不能使用 "0.0.0.0"。
    pub advertised_ip: String,

    /// 公网端口
    ///
    /// ICE 服务对外宣告的端口号，通常与绑定端口相同。
    #[serde(default = "default_advertised_port")]
    pub advertised_port: u16,
}

fn default_advertised_port() -> u16 {
    3478
}

impl Default for IceBindConfig {
    fn default() -> Self {
        Self {
            ip: "0.0.0.0".to_string(),
            port: 3478,
            advertised_ip: "127.0.0.1".to_string(),
            advertised_port: 3478,
        }
    }
}
