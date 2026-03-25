//! MFR (Manufacturer Registry) HTTP 服务实现
//!
//! 提供厂商注册、域名验证、Actor 包发布与查询的 HTTP API 服务

use crate::service::HttpRouterService;
use actrix_mfr::{
    MfrManager,
    handlers::{MfrState, create_public_router},
};
use anyhow::Result;
use async_trait::async_trait;
use axum::Router;
use platform::config::ActrixConfig;
use platform::{ServiceInfo, ServiceType};
use std::sync::Arc;

/// MFR HTTP 服务实现
#[derive(Debug)]
pub struct MfrService {
    info: ServiceInfo,
    #[allow(dead_code)]
    config: ActrixConfig,
}

impl MfrService {
    pub fn new(config: ActrixConfig) -> Self {
        Self {
            info: ServiceInfo::new(
                "MFR Service",
                ServiceType::Mfr,
                Some("Manufacturer Registry - 厂商注册和 Actor 包签名服务".to_string()),
                &config,
            ),
            config,
        }
    }
}

#[async_trait]
impl HttpRouterService for MfrService {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn info_mut(&mut self) -> &mut ServiceInfo {
        &mut self.info
    }

    async fn build_router(&mut self) -> Result<Router> {
        platform::recording::info!("Building MFR router");

        let pool = platform::storage::db::get_database().get_pool().clone();
        let nonce_retain_secs = self.config.services.mfr.nonce_retain_secs;
        let manager = MfrManager::new(pool).with_nonce_retain_secs(nonce_retain_secs);
        let state = MfrState {
            manager: Arc::new(manager),
        };
        let mfr_router = create_public_router(state);
        let router = Router::new().merge(mfr_router);

        platform::recording::info!("MFR router built successfully");
        Ok(router)
    }

    fn route_prefix(&self) -> &str {
        "/mfr"
    }
}
