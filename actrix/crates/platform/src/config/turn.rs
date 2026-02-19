use serde::{Deserialize, Serialize};

/// TURN 服务配置
///
/// TURN 中继服务的专用配置参数。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TurnConfig {
    /// 公网 IP 地址
    ///
    /// TURN 服务对外宣告的公网 IP 地址。客户端通过此地址进行中继通信。
    /// 必须是真实可路由的公网 IP，不能使用 "0.0.0.0"。
    pub advertised_ip: String,

    /// 公网端口
    ///
    /// TURN 服务对外宣告的端口号，通常与绑定端口相同。
    pub advertised_port: u16,

    /// 中继端口范围
    ///
    /// TURN 服务用于数据中继的 UDP 端口范围。
    /// 格式：开始端口-结束端口，如 "49152-65535"。
    /// 范围越大，可支持的并发中继会话越多。
    pub relay_port_range: String,

    /// TURN 认证域
    ///
    /// TURN 服务的认证域名，用于 TURN 协议的认证机制。
    pub realm: String,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            advertised_ip: "127.0.0.1".to_string(),
            advertised_port: 3478,
            relay_port_range: "49152-65535".to_string(),
            realm: "actor-rtc.local".to_string(),
        }
    }
}
