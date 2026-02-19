//! # aux-servers
//!
//! WebRTC 辅助服务器集合，包括信令服务、STUN 和 TURN 服务

pub mod service;

// Re-export commonly used types
pub use platform::config::ActrixConfig;
pub use service::{ServiceContainer, ServiceManager};
