//! 服务类型定义
//!
//! 定义了系统支持的各种服务类型

use serde::{Deserialize, Serialize};

/// 服务类型枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ServiceType {
    #[default]
    Authority, // 权限认证服务
    Signaling, // 信令服务
    Turn,      // TURN 服务
}
