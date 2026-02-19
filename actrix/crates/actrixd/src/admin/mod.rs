//! 内置 admin 运行时编排
//!
//! 保持现有 supervisor 双向能力不变：
//! - 启动本地 Supervisord gRPC server（供外部 supervisor 回连）
//! - 启动 Admin client（向外部 supervisor 注册与上报）

use crate::error::{Error, Result};
use crate::service::{AdminApiGrpcService, ServiceManager};
use actrix_sdk::control::{AdminClient, AdminConfig};
use platform::{ServiceCollector, config::ActrixConfig, config::SupervisorConfig};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// 内置 admin 运行时。
///
/// 这是一个内部组织层，不改变现有外部配置与协议行为。
#[derive(Debug, Clone)]
pub struct AdminRuntime {
    supervisor_config: SupervisorConfig,
    sqlite_path: PathBuf,
    location_tag: String,
    service_collector: ServiceCollector,
    client_enabled: bool,
}

impl AdminRuntime {
    /// 根据 ActrixConfig 构建 AdminRuntime。
    ///
    /// 返回 `None` 表示未配置 `[supervisor]`，不启用内置 admin。
    pub fn from_config(config: &ActrixConfig, service_manager: &ServiceManager) -> Option<Self> {
        let supervisor_config = config.supervisor.clone()?;
        Some(Self {
            supervisor_config,
            sqlite_path: config.sqlite_path.clone(),
            location_tag: config.location_tag.clone(),
            service_collector: service_manager.service_collector(),
            client_enabled: config.is_supervisor_enabled(),
        })
    }

    /// 启动本地 Supervisord gRPC server（SupervisedService）。
    pub async fn start_server(&self, shutdown_tx: broadcast::Sender<()>) -> Result<JoinHandle<()>> {
        if self.supervisor_config.shared_secret().trim().is_empty() {
            return Err(Error::service_startup(
                "supervisor.client.shared_secret cannot be empty, refusing to start Supervisord gRPC service"
                    .to_string(),
            ));
        }

        info!("启动 Supervisord gRPC 服务器...");
        let bind_addr_str = self.supervisor_config.supervisord.bind_addr();
        let bind_addr: SocketAddr = bind_addr_str.parse().map_err(|e| {
            Error::service_startup(format!(
                "Failed to parse supervisord bind address {bind_addr_str}: {e}"
            ))
        })?;

        let mut grpc_service = AdminApiGrpcService::new(
            self.supervisor_config.clone(),
            self.sqlite_path.clone(),
            self.location_tag.clone(),
            self.service_collector.clone(),
        );

        grpc_service
            .start(bind_addr, shutdown_tx)
            .await
            .map_err(|e| Error::service_startup(format!("Supervisord gRPC 初始化失败: {e}")))
    }

    /// 启动 Admin client（注册 + 状态上报）。
    ///
    /// 与历史行为一致：仅在 `config.is_supervisor_enabled()` 时启动。
    pub fn start_client(&self) -> Option<JoinHandle<()>> {
        if !self.client_enabled {
            return None;
        }

        let client_config = self.build_client_config();
        let service_collector = self.service_collector.clone();

        info!("Starting Admin client (register and status reporting)...");
        Some(tokio::spawn(async move {
            match AdminClient::new(client_config.clone(), service_collector) {
                Ok(mut client) => {
                    if let Err(e) = client.connect().await {
                        warn!("Admin client connect failed: {}", e);
                        return;
                    }

                    if let Err(e) = client.register_node().await {
                        warn!("Register node failed: {}", e);
                    } else {
                        info!("✅ Node registered successfully with services");
                    }

                    if let Err(e) = client.start_status_reporting().await {
                        warn!("Start status reporting failed: {}", e);
                    } else {
                        info!("✅ Status reporting started");
                    }
                }
                Err(e) => warn!("Create admin client failed: {}", e),
            }
        }))
    }

    fn build_client_config(&self) -> AdminConfig {
        let supervisor_cfg = &self.supervisor_config;
        let supervisord_cfg = &supervisor_cfg.supervisord;

        AdminConfig {
            node_id: supervisor_cfg.node_id().to_string(),
            name: Some(supervisor_cfg.node_name().to_string()),
            location_tag: self.location_tag.clone(),
            endpoint: supervisor_cfg.endpoint().to_string(),
            agent_addr: supervisord_cfg.advertised_addr(),
            connect_timeout_secs: supervisor_cfg.connect_timeout_secs,
            status_report_interval_secs: supervisor_cfg.status_report_interval_secs,
            health_check_interval_secs: supervisor_cfg.health_check_interval_secs,
            enable_tls: supervisor_cfg.enable_tls,
            tls_domain: supervisor_cfg.tls_domain.clone(),
            client_cert: supervisor_cfg.client_cert.clone(),
            client_key: supervisor_cfg.client_key.clone(),
            ca_cert: supervisor_cfg.ca_cert.clone(),
            shared_secret: Some(supervisor_cfg.shared_secret().to_string()),
            max_clock_skew_secs: supervisor_cfg.max_clock_skew_secs,
            location: None,
            service_tags: Vec::new(),
        }
    }
}
