//! Signaling WebSocket服务实现

use crate::service::HttpRouterService;
use anyhow::Result;
use async_trait::async_trait;
use axum::{Router, routing::get};
use platform::config::ActrixConfig;
use platform::{ServiceInfo, ServiceType};
use signaling::create_signaling_router_with_config;

/// Signaling WebSocket服务实现
#[derive(Debug)]
pub struct SignalingService {
    info: ServiceInfo,
    config: ActrixConfig,
}

impl SignalingService {
    pub fn new(config: ActrixConfig) -> Self {
        Self {
            info: ServiceInfo::new(
                "Signaling Service",
                ServiceType::Signaling,
                Some("WebRTC signaling service with WebSocket support".to_string()),
                &config,
            ),
            config,
        }
    }
}

#[async_trait]
impl HttpRouterService for SignalingService {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn info_mut(&mut self) -> &mut ServiceInfo {
        &mut self.info
    }

    async fn build_router(&mut self) -> Result<Router> {
        platform::recording::info!("Building Signaling router");
        let signaling_router = create_signaling_router_with_config(&self.config).await?;

        let router = Router::new()
            .route("/health", get(|| async { "Signaling is healthy" }))
            .merge(signaling_router);

        platform::recording::info!("Signaling router built successfully");
        Ok(router)
    }

    fn route_prefix(&self) -> &str {
        "/signaling"
    }
}
