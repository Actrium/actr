//! 服务状态定义
//!
//! 定义了服务状态枚举，用于表示各种服务的运行状态

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceState {
    Unknown,
    Running(String),
    Error(String),
}
