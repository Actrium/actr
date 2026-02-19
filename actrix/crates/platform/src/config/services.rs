//! 服务配置集合

use super::ais::AisConfig;
use super::signaling::SignalingConfig;
use serde::{Deserialize, Serialize};

/// 所有服务的配置集合
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ServicesConfig {
    /// KS (Key Server) 服务配置
    #[serde(default)]
    pub ks: Option<ks::KsServiceConfig>,

    /// AIS (Actor Identity Service) 服务配置
    #[serde(default)]
    pub ais: Option<AisConfig>,

    /// Signaling 服务配置
    #[serde(default)]
    pub signaling: Option<SignalingConfig>,
    // 注意：STUN/TURN 不依赖 KS，保持原有配置方式
}
