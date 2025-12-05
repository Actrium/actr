//! KS (Key Server) HTTP 服务实现
//!
//! 提供椭圆曲线密钥生成和管理的 HTTP API 服务

use crate::service::HttpRouterService;
use actrix_common::config::ActrixConfig;
use actrix_common::storage::nonce::SqliteNonceStorage;
use actrix_common::{ServiceInfo, ServiceType};
use anyhow::Result;
use async_trait::async_trait;
use axum::Router;
use ks::{create_ks_state, create_router};
use tracing::info;

/// KS HTTP 服务实现
#[derive(Debug)]
pub struct KsHttpService {
    info: ServiceInfo,
    config: ActrixConfig,
}

impl KsHttpService {
    pub fn new(config: ActrixConfig) -> Self {
        Self {
            info: ServiceInfo::new(
                "KS Service",
                ServiceType::Ks,
                Some("Key Server - 椭圆曲线密钥生成和管理服务".to_string()),
                &config,
            ),
            config,
        }
    }
}

#[async_trait]
impl HttpRouterService for KsHttpService {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn info_mut(&mut self) -> &mut ServiceInfo {
        &mut self.info
    }

    async fn build_router(&mut self) -> Result<Router> {
        info!("Building KS router");

        // 创建 KS 状态
        let ks_service_config = self
            .config
            .services
            .ks
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("KS service configuration not found"))?;

        // 创建 nonce storage 实例（用于防重放攻击）
        // 使用 sqlite_path 作为目录路径，内部会自动拼接 nonce.db
        let nonce_storage = SqliteNonceStorage::new_async(&self.config.sqlite_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create nonce storage: {e}"))?;

        // 创建 KS state（注入 nonce storage 和 shared key）
        let ks_state = create_ks_state(
            ks_service_config,
            nonce_storage,
            &self.config.actrix_shared_key,
            &self.config.sqlite_path,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create KS state: {e}"))?;

        // 获取 KS 路由器
        let router = create_router(ks_state);

        info!("KS router built successfully");
        Ok(router)
    }

    fn route_prefix(&self) -> &str {
        "/ks"
    }
}
