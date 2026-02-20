//! 内置 admin 运行时编排
//!
//! 保持现有 admin 双向能力不变：
//! - 启动本地 AdminApi gRPC server（供外部 admin 回连）
//! - 启动 Admin client（向外部 admin 注册与上报）

use crate::error::{Error, Result};
use crate::service::{AdminApiGrpcService, ServiceManager};
use actrix_sdk::control::{AdminClient, AdminConfig as AdminClientConfig};
use platform::{ServiceCollector, config::ActrixConfig, config::AdminPlaneConfig};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// 内置 admin 运行时。
///
/// 这是一个内部组织层，不改变现有外部配置与协议行为。
#[derive(Debug, Clone)]
pub struct AdminRuntime {
    admin_config: AdminPlaneConfig,
    sqlite_path: PathBuf,
    location_tag: String,
    service_collector: ServiceCollector,
    client_enabled: bool,
}

impl AdminRuntime {
    /// 根据 ActrixConfig 构建 AdminRuntime。
    ///
    /// 返回 `None` 表示未配置 `[admin]`，不启用内置 admin。
    pub fn from_config(config: &ActrixConfig, service_manager: &ServiceManager) -> Option<Self> {
        let admin_config = config.admin.clone()?;
        Some(Self {
            admin_config,
            sqlite_path: config.sqlite_path.clone(),
            location_tag: config.location_tag.clone(),
            service_collector: service_manager.service_collector(),
            client_enabled: config.is_admin_enabled(),
        })
    }

    /// 启动本地 AdminApi gRPC server（NodeAdminService）。
    pub async fn start_server(&self, shutdown_tx: broadcast::Sender<()>) -> Result<JoinHandle<()>> {
        if self.admin_config.shared_secret().trim().is_empty() {
            return Err(Error::service_startup(
                "admin.client.shared_secret cannot be empty, refusing to start AdminApi gRPC service"
                    .to_string(),
            ));
        }

        platform::recording::info!("启动 AdminApi gRPC 服务器...");
        let bind_addr_str = self.admin_config.api.bind_addr();
        let bind_addr: SocketAddr = bind_addr_str.parse().map_err(|e| {
            Error::service_startup(format!(
                "Failed to parse admin_api bind address {bind_addr_str}: {e}"
            ))
        })?;

        let mut grpc_service = AdminApiGrpcService::new(
            self.admin_config.clone(),
            self.sqlite_path.clone(),
            self.location_tag.clone(),
            self.service_collector.clone(),
        );

        grpc_service
            .start(bind_addr, shutdown_tx)
            .await
            .map_err(|e| Error::service_startup(format!("AdminApi gRPC 初始化失败: {e}")))
    }

    /// 启动 Admin client（注册 + 状态上报）。
    ///
    /// 与历史行为一致：仅在 `config.is_admin_enabled()` 时启动。
    pub fn start_client(&self) -> Option<JoinHandle<()>> {
        if !self.client_enabled {
            return None;
        }

        let client_config = self.build_client_config();
        let service_collector = self.service_collector.clone();

        platform::recording::info!("Starting Admin client (register and status reporting)...");
        Some(tokio::spawn(async move {
            match AdminClient::new(client_config.clone(), service_collector) {
                Ok(mut client) => {
                    if let Err(e) = client.connect().await {
                        platform::recording::warn!("Admin client connect failed: {}", e);
                        return;
                    }

                    if let Err(e) = client.register_node().await {
                        platform::recording::warn!("Register node failed: {}", e);
                    } else {
                        platform::recording::info!("✅ Node registered successfully with services");
                    }

                    if let Err(e) = client.start_status_reporting().await {
                        platform::recording::warn!("Start status reporting failed: {}", e);
                    } else {
                        platform::recording::info!("✅ Status reporting started");
                    }
                }
                Err(e) => platform::recording::warn!("Create admin client failed: {}", e),
            }
        }))
    }

    fn build_client_config(&self) -> AdminClientConfig {
        let admin_cfg = &self.admin_config;
        let admin_api_cfg = &admin_cfg.api;

        AdminClientConfig {
            node_id: admin_cfg.node_id().to_string(),
            name: Some(admin_cfg.node_name().to_string()),
            location_tag: self.location_tag.clone(),
            endpoint: admin_cfg.endpoint().to_string(),
            agent_addr: admin_api_cfg.advertised_addr(),
            connect_timeout_secs: admin_cfg.connect_timeout_secs,
            status_report_interval_secs: admin_cfg.status_report_interval_secs,
            health_check_interval_secs: admin_cfg.health_check_interval_secs,
            enable_tls: admin_cfg.enable_tls,
            tls_domain: admin_cfg.tls_domain.clone(),
            client_cert: admin_cfg.client_cert.clone(),
            client_key: admin_cfg.client_key.clone(),
            ca_cert: admin_cfg.ca_cert.clone(),
            shared_secret: Some(admin_cfg.shared_secret().to_string()),
            max_clock_skew_secs: admin_cfg.max_clock_skew_secs,
            location: None,
            service_tags: Vec::new(),
        }
    }
}
