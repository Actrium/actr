//! Actor-RTC 信令服务
//!
//! 基于 protobuf SignalingEnvelope 协议的 WebSocket 信令服务
//!
//! # 模块结构
//!
//! ## 核心模块
//! - [`server`]: WebSocket 服务器和协议处理
//! - [`service_registry`][]: 服务注册与发现
//! - [`compatibility_cache`][]: 全局兼容性缓存
//!
//! ## 扩展模块
//! - [`presence`] - Presence 订阅管理
//! - [`load_balancer`] - 负载均衡算法
//! - [`geo`] - 地理位置和距离计算

pub mod actr_type_utils;
pub mod ais_client;
pub mod compatibility_cache;
pub mod geo;
pub mod load_balancer;
pub mod presence;
pub mod ratelimit;
pub mod server;
pub mod service_registry;
pub mod service_registry_storage;
#[cfg(feature = "opentelemetry")]
pub mod trace;

// Axum router integration
pub mod axum_router;

pub use axum_router::{create_signaling_router, create_signaling_router_with_config};

// Re-export commonly used types
pub use compatibility_cache::GlobalCompatibilityCache;
pub use load_balancer::LoadBalancer;
pub use presence::PresenceManager;
pub use server::{ClientConnection, SignalingServer, SignalingServerHandle};
pub use service_registry::{ServiceInfo, ServiceRegistry};

// Export WebSocket handler
pub use server::handle_websocket_connection;
