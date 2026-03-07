use serde::{Deserialize, Serialize};

/// TURN 服务配置
///
/// TURN 中继服务的专用配置参数。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct TurnConfig {
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
            relay_port_range: "49152-65535".to_string(),
            realm: "actrix.local".to_string(),
        }
    }
}
