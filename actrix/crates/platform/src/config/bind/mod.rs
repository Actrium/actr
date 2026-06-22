pub mod http;
pub mod ice;

pub use crate::config::bind::http::HttpBindConfig;
pub use crate::config::bind::ice::IceBindConfig;
use serde::{Deserialize, Serialize};

/// 网络绑定配置
///
/// 定义不同类型服务的网络绑定参数。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BindConfig {
    /// HTTP/HTTPS 服务绑定配置（可选）
    ///
    /// 统一的 HTTP 绑定。配置了 cert+key 即为 HTTPS，否则为 HTTP。
    pub http: Option<HttpBindConfig>,

    /// ICE 服务绑定配置
    ///
    /// 用于 STUN/TURN 服务的 UDP 绑定配置。
    pub ice: IceBindConfig,
}

impl Default for BindConfig {
    fn default() -> Self {
        Self {
            http: Some(HttpBindConfig::default()),
            ice: IceBindConfig::default(),
        }
    }
}
