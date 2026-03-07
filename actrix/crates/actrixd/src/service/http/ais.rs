//! AIS (Actor Identity Service) HTTP 服务实现
//!
//! 提供 ActrId 注册和 Token 签发的 HTTP API 服务

use crate::service::HttpRouterService;
use ais::create_ais_router_with_counters;
use anyhow::Result;
use async_trait::async_trait;
use axum::Router;
use platform::config::ActrixConfig;
use platform::monitoring::ServiceCounters;
use platform::{ServiceInfo, ServiceType};
use std::sync::Arc;

/// AIS HTTP 服务实现
#[derive(Debug)]
pub struct AisService {
    info: ServiceInfo,
    config: ActrixConfig,
    cancel: tokio_util::sync::CancellationToken,
    /// Service-level counters for metrics collection.
    counters: Option<Arc<ServiceCounters>>,
}

impl AisService {
    #[allow(dead_code)]
    pub fn new(config: ActrixConfig, cancel: tokio_util::sync::CancellationToken) -> Self {
        Self {
            info: ServiceInfo::new(
                "AIS Service",
                ServiceType::Ais,
                Some("Actor Identity Service - ActrId 注册和凭证签发服务".to_string()),
                &config,
            ),
            config,
            cancel,
            counters: None,
        }
    }

    /// Attach service-level counters.
    pub fn set_counters(&mut self, counters: Arc<ServiceCounters>) {
        self.info.set_counters(counters.clone());
        self.counters = Some(counters);
    }
}

#[async_trait]
impl HttpRouterService for AisService {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn info_mut(&mut self) -> &mut ServiceInfo {
        &mut self.info
    }

    async fn build_router(&mut self) -> Result<Router> {
        platform::recording::info!("Building AIS router");

        // 获取 AIS 配置
        let ais_config = self
            .config
            .services
            .ais
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("AIS config not found"))?;

        // 创建 AIS 路由器（传递配置和计数器）
        let ais_router = create_ais_router_with_counters(
            ais_config,
            &self.config,
            self.cancel.clone(),
            self.counters.clone(),
        )
        .await?;

        let router = Router::new().merge(ais_router);

        platform::recording::info!("AIS router built successfully");
        Ok(router)
    }

    fn route_prefix(&self) -> &str {
        "/ais"
    }
}
