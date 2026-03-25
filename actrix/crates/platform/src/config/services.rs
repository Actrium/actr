//! 服务配置集合

use super::ais::AisConfig;
use super::signaling::SignalingConfig;
use serde::{Deserialize, Serialize};

/// MFR (Manufacturer Registry) 配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MfrConfig {
    /// How long (in seconds) to retain expired/used publish nonces for auditing.
    /// Default: 86400 (24 hours).
    #[serde(default = "default_nonce_retain_secs")]
    pub nonce_retain_secs: i64,
}

fn default_nonce_retain_secs() -> i64 {
    86400 // 24 hours
}

impl Default for MfrConfig {
    fn default() -> Self {
        Self {
            nonce_retain_secs: default_nonce_retain_secs(),
        }
    }
}

/// 所有服务的配置集合
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ServicesConfig {
    /// Signer 服务配置
    #[serde(default)]
    pub signer: Option<signer::SignerServiceConfig>,

    /// AIS (Actor Identity Service) 服务配置
    #[serde(default)]
    pub ais: Option<AisConfig>,

    /// Signaling 服务配置
    #[serde(default)]
    pub signaling: Option<SignalingConfig>,
    // 注意：STUN/TURN 不依赖 Signer，保持原有配置方式
    /// MFR (Manufacturer Registry) 服务配置
    #[serde(default)]
    pub mfr: MfrConfig,
}
