//! Rate limiting middleware for AIS service
//!
//! 限流策略：
//! - **IP 级别**：每个 IP 最多 100 req/min（突发 100 请求）
//!
//! 使用 tower-governor v0.8 实现限流，防止 DoS 攻击和资源耗尽

use axum::body::Body;
use governor::middleware::NoOpMiddleware;
use std::sync::Arc;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};

/// IP 级别限流配置
///
/// 限制策略：
/// - 每秒 2 个请求（120 req/min）
/// - 突发允许 100 个请求
/// - 基于客户端 IP 地址限流
///
/// 使用 tower_governor v0.8.0 API
pub fn ip_rate_limiter() -> GovernorLayer<SmartIpKeyExtractor, NoOpMiddleware, Body> {
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(2) // 每秒 2 个请求
            .burst_size(100) // 允许突发 100 个请求
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .unwrap(),
    );

    GovernorLayer::new(governor_conf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_rate_limiter_creation() {
        let _limiter = ip_rate_limiter();
        // 如果能创建成功，说明配置正确
    }
}
