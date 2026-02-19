pub mod http;
pub mod https;
pub mod ice;

pub use crate::config::bind::http::HttpBindConfig;
pub use crate::config::bind::https::HttpsBindConfig;
pub use crate::config::bind::ice::IceBindConfig;
use serde::{Deserialize, Serialize};

/// 网络绑定配置
///
/// 定义不同类型服务的网络绑定参数。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BindConfig {
    /// HTTP 服务绑定配置（可选）
    ///
    /// 用于开发环境或内部服务。生产环境建议使用 HTTPS。
    pub http: Option<HttpBindConfig>,

    /// HTTPS 服务绑定配置（可选）
    ///
    /// 提供加密的 HTTP 服务，包括 API 接口和 WebSocket 升级。
    /// 生产环境强烈建议配置。
    pub https: Option<HttpsBindConfig>,

    /// ICE 服务绑定配置
    ///
    /// 用于 STUN/TURN 服务的 UDP 绑定配置。
    pub ice: IceBindConfig,
}

impl Default for BindConfig {
    fn default() -> Self {
        Self {
            http: Some(HttpBindConfig::default()),
            https: Some(HttpsBindConfig::default()),
            ice: IceBindConfig::default(),
        }
    }
}
