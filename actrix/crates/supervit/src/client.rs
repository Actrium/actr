//! gRPC client for supervisor communication

use crate::config::SupervitConfig;
use crate::error::{Result, SupervitError};
use crate::metrics::collect_system_metrics;
use crate::nonce_auth::generate_credential;
use crate::realm::get_max_realm_version;
use crate::{
    HealthCheckRequest, HealthCheckResponse, RegisterNodeRequest, RegisterNodeResponse,
    ReportRequest, ReportResponse, ServiceAdvertisement, ServiceAdvertisementStatus,
    SupervisorServiceClient as GrpcSupervisorClient,
};
use actrix_common::ServiceCollector;

use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::time::interval;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tracing::{debug, error, info, warn};

/// Supervit gRPC 客户端
pub struct SupervitClient {
    config: SupervitConfig,
    client: Option<GrpcSupervisorClient<Channel>>,
    shared_secret: Vec<u8>,    // hex decoded shared secret
    service_tags: Vec<String>, // normalized service tags
    service_collector: ServiceCollector,
}

impl SupervitClient {
    /// 创建新的 supervit 客户端（使用服务收集器）
    pub fn new(config: SupervitConfig, service_collector: ServiceCollector) -> Result<Self> {
        config.validate()?;

        // 解码共享密钥
        let shared_secret = if let Some(ref secret_hex) = config.shared_secret {
            hex::decode(secret_hex)
                .map_err(|e| SupervitError::Config(format!("Invalid shared_secret hex: {e}")))?
        } else {
            return Err(SupervitError::Config(
                "shared_secret is required for authentication".to_string(),
            ));
        };

        let mut service_tags = config.service_tags.clone();
        service_tags.sort();
        service_tags.dedup();

        Ok(Self {
            config,
            client: None,
            shared_secret,
            service_tags,
            service_collector,
        })
    }

    /// 连接到 supervisor 服务器
    pub async fn connect(&mut self) -> Result<()> {
        info!(
            "Connecting to supervisor at: {} (node: {})",
            self.config.endpoint, self.config.node_id
        );

        let mut endpoint = Endpoint::from_shared(self.config.endpoint.clone())
            .map_err(|e| SupervitError::Config(format!("Invalid server address: {e}")))?
            .timeout(Duration::from_secs(self.config.connect_timeout_secs))
            .connect_timeout(Duration::from_secs(self.config.connect_timeout_secs));

        // 如果启用 TLS，配置 TLS/mTLS
        if self.config.enable_tls {
            let tls_config = self.build_tls_config()?;
            endpoint = endpoint
                .tls_config(tls_config)
                .map_err(|e| SupervitError::Config(format!("TLS configuration error: {e}")))?;
            info!("TLS enabled for connection");
        }

        let channel = endpoint.connect().await?;
        self.client = Some(GrpcSupervisorClient::new(channel));

        info!("Successfully connected to supervisor");
        Ok(())
    }

    /// 构建 TLS 配置（支持 mTLS）
    fn build_tls_config(&self) -> Result<ClientTlsConfig> {
        // TLS 域名是必需的
        let tls_domain = self.config.tls_domain.as_ref().ok_or_else(|| {
            SupervitError::Config("tls_domain is required when enable_tls is true".to_string())
        })?;

        let mut tls_config = ClientTlsConfig::new().domain_name(tls_domain);

        debug!("Configuring TLS with domain: {}", tls_domain);

        // 加载 CA 证书（验证服务端证书）
        if let Some(ca_cert_path) = &self.config.ca_cert {
            debug!("Loading CA certificate from: {}", ca_cert_path);
            let ca_cert_pem = std::fs::read(ca_cert_path).map_err(|e| {
                SupervitError::Config(format!(
                    "Failed to read CA certificate from {ca_cert_path}: {e}"
                ))
            })?;

            let ca_cert = Certificate::from_pem(ca_cert_pem);
            tls_config = tls_config.ca_certificate(ca_cert);
            info!("CA certificate loaded for server verification");
        }

        // 加载客户端证书和私钥（mTLS）
        if let (Some(cert_path), Some(key_path)) =
            (&self.config.client_cert, &self.config.client_key)
        {
            debug!("Loading client certificate from: {}", cert_path);
            debug!("Loading client private key from: {}", key_path);

            let client_cert_pem = std::fs::read(cert_path).map_err(|e| {
                SupervitError::Config(format!(
                    "Failed to read client certificate from {cert_path}: {e}"
                ))
            })?;

            let client_key_pem = std::fs::read(key_path).map_err(|e| {
                SupervitError::Config(format!(
                    "Failed to read client private key from {key_path}: {e}"
                ))
            })?;

            let identity = Identity::from_pem(client_cert_pem, client_key_pem);
            tls_config = tls_config.identity(identity);
            info!("mTLS enabled: client certificate and key loaded");
        } else if self.config.client_cert.is_some() || self.config.client_key.is_some() {
            // 如果只配置了证书或私钥其中一个，报错
            return Err(SupervitError::Config(
                "Both client_cert and client_key must be provided for mTLS".to_string(),
            ));
        }

        Ok(tls_config)
    }

    /// Execute status report
    pub async fn report(&mut self) -> Result<ReportResponse> {
        let client = self
            .client
            .as_mut()
            .ok_or(SupervitError::ConnectionClosed)?;

        let location_tag = self.config.location_tag.clone();
        let name = self.config.name.clone().unwrap_or_else(|| {
            std::env::var("NODE_NAME").unwrap_or_else(|_| self.config.node_id.clone())
        });

        let request = Self::create_report_request(
            &self.config.node_id,
            &location_tag,
            &name,
            &self.shared_secret,
            self.service_collector.clone(),
        )
        .await?;

        debug!("Sending status report for node: {}", self.config.node_id);

        let response = client.report(request).await?.into_inner();

        debug!(
            "Status report acknowledged, next interval: {}s",
            response.next_report_interval_secs
        );

        Ok(response)
    }

    /// Register node information (including supervisord advertised address)
    pub async fn register_node(&mut self) -> Result<RegisterNodeResponse> {
        let location_tag = self.config.location_tag.clone();
        let name = self.config.name.clone().unwrap_or_else(|| {
            std::env::var("NODE_NAME").unwrap_or_else(|_| self.config.node_id.clone())
        });

        let services = self.build_service_advertisements().await;
        let power_reserve_level_init = Self::read_power_reserve_level().await;
        let fingerprint = self.build_registration_fingerprint(&services, power_reserve_level_init);
        let payload = format!("register:{}:{}", self.config.node_id, fingerprint);
        let credential = generate_credential(&self.shared_secret, payload.as_bytes())?;

        let request = RegisterNodeRequest {
            node_id: self.config.node_id.clone(),
            name,
            location_tag,
            version: env!("CARGO_PKG_VERSION").to_string(),
            agent_addr: self.config.agent_addr.clone(),
            credential,
            location: self.config.location.clone(),
            service_tags: self.service_tags.clone(),
            power_reserve_level_init,
            services,
        };

        let client = self
            .client
            .as_mut()
            .ok_or(SupervitError::ConnectionClosed)?;

        debug!(
            "Registering node {} with advertised address {}",
            self.config.node_id, self.config.agent_addr
        );

        let response = client.register_node(request).await?.into_inner();
        debug!(
            "Register response received, heartbeat interval: {}s",
            response.heartbeat_interval_secs
        );
        Ok(response)
    }

    /// 启动状态上报循环
    pub async fn start_status_reporting(&mut self) -> Result<()> {
        let mut interval_secs = self.config.status_report_interval_secs;
        let shared_secret = self.shared_secret.clone();
        let mut report_config = self.config.clone();
        report_config.status_report_interval_secs = interval_secs;
        report_config.shared_secret = Some(hex::encode(&shared_secret));
        let node_id = report_config.node_id.clone();
        let location_tag = report_config.location_tag.clone();
        let name = report_config
            .name
            .clone()
            .unwrap_or_else(|| std::env::var("NODE_NAME").unwrap_or_else(|_| node_id.clone()));
        let service_collector = self.service_collector.clone();

        // 启动状态上报任务
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs));

            // 创建独立的客户端连接
            let mut client =
                match SupervitClient::new(report_config.clone(), service_collector.clone()) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to create report client: {}", e);
                        return;
                    }
                };

            if let Err(e) = client.connect().await {
                error!("Failed to connect report client: {}", e);
                return;
            }

            loop {
                ticker.tick().await;

                match Self::create_report_request(
                    &node_id,
                    &location_tag,
                    &name,
                    &shared_secret,
                    service_collector.clone(),
                )
                .await
                {
                    Ok(request) => {
                        debug!("Sending status report for node: {}", node_id);
                        match client.client.as_mut() {
                            Some(grpc_client) => match grpc_client.report(request).await {
                                Ok(response) => {
                                    let resp = response.into_inner();
                                    debug!("Status report acknowledged");
                                    // 动态调整上报间隔
                                    if resp.next_report_interval_secs > 0
                                        && resp.next_report_interval_secs as u64 != interval_secs
                                    {
                                        interval_secs = resp.next_report_interval_secs as u64;
                                        ticker = interval(Duration::from_secs(interval_secs));
                                        info!("Adjusted report interval to {}s", interval_secs);
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to send status report: {}", e);
                                }
                            },
                            None => {
                                error!("Client not connected");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to create status report: {}", e);
                    }
                }
            }
            info!("Status reporting stopped");
        });

        Ok(())
    }

    /// 执行健康检查
    pub async fn health_check(&mut self) -> Result<HealthCheckResponse> {
        let client = self
            .client
            .as_mut()
            .ok_or(SupervitError::ConnectionClosed)?;

        // 构造请求负载（用于签名）
        let payload = format!("health_check:{}", self.config.node_id);

        // 生成认证凭证
        let credential = generate_credential(&self.shared_secret, payload.as_bytes())?;

        let request = HealthCheckRequest {
            node_id: self.config.node_id.clone(),
            credential,
        };

        debug!("Sending health check request with nonce-auth credential");

        let response = client.health_check(request).await?.into_inner();

        debug!(
            "Health check successful, latency: {}ms",
            response.latency_ms
        );

        Ok(response)
    }

    /// 创建状态报告请求（带认证凭证）
    async fn create_report_request(
        node_id: &str,
        location_tag: &str,
        name: &str,
        shared_secret: &[u8],
        service_collector: ServiceCollector,
    ) -> Result<ReportRequest> {
        let metrics = collect_system_metrics().await?;

        // Get service statuses from collector
        let services = service_collector.all_statuses().await;

        // 获取 power_reserve_level（0-5 级别）
        let power_reserve_level = match pwrzv::get_power_reserve_level_direct().await {
            Ok(level) => {
                let clamped = Self::clamp_power_reserve(level);
                debug!("Power reserve level: {:.2} -> {}", level, clamped);
                clamped
            }
            Err(e) => {
                warn!(
                    "Failed to get power reserve level from pwrzv: {}, using default 0",
                    e
                );
                0
            }
        };

        // 获取本地最大 realm 版本号（用于 Supervisor 检测同步滞后）
        let realm_sync_version = get_max_realm_version().await.unwrap_or(0);

        let timestamp = chrono::Utc::now().timestamp();

        // 构造请求负载
        let payload = format!("report:{node_id}:{timestamp}");

        // 生成认证凭证
        let credential = generate_credential(shared_secret, payload.as_bytes())?;

        Ok(ReportRequest {
            node_id: node_id.to_string(),
            timestamp,
            location_tag: location_tag.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            name: name.to_string(),
            power_reserve_level,
            metrics: Some(metrics),
            services,
            credential,
            realm_sync_version,
        })
    }

    /// Build static service advertisement list for registration
    async fn build_service_advertisements(&self) -> Vec<ServiceAdvertisement> {
        let mut base_tags = self.service_tags.clone();
        base_tags.sort();
        base_tags.dedup();

        // Get service statuses from collector and convert to ServiceAdvertisement
        let statuses = self.service_collector.all_statuses().await;
        statuses
            .into_iter()
            .map(|status| {
                // Convert ServiceStatus to ServiceAdvertisement
                // Map is_healthy to ServiceAdvertisementStatus
                let status_enum = if status.is_healthy {
                    ServiceAdvertisementStatus::Running as i32
                } else {
                    ServiceAdvertisementStatus::Error as i32
                };
                ServiceAdvertisement {
                    name: status.name,
                    r#type: status.r#type,
                    domain_name: status.domain.unwrap_or_default(),
                    port_info: status.port.map(|p| p.to_string()).unwrap_or_default(),
                    status: status_enum,
                    description: None,
                    url: status.url,
                    tags: base_tags.clone(),
                }
            })
            .collect()
    }

    /// Compute a stable fingerprint for static registration payload
    fn build_registration_fingerprint(
        &self,
        services: &[ServiceAdvertisement],
        power_reserve_level_init: Option<u32>,
    ) -> String {
        let mut tags = self.service_tags.clone();
        tags.sort();
        tags.dedup();

        let mut service_entries = services
            .iter()
            .map(|svc| {
                let mut svc_tags = svc.tags.clone();
                svc_tags.sort();
                format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    svc.name,
                    svc.r#type,
                    svc.domain_name,
                    svc.port_info,
                    svc.status,
                    svc.url.clone().unwrap_or_default(),
                    svc_tags.join(",")
                )
            })
            .collect::<Vec<_>>();
        service_entries.sort();

        let location = self.config.location.clone().unwrap_or_default();
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.config.node_id,
            self.config.agent_addr,
            self.config.location_tag,
            location,
            power_reserve_level_init.unwrap_or(0),
            tags.join(","),
            service_entries.join(";")
        );

        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Read current power reserve level and clamp to 0-5
    async fn read_power_reserve_level() -> Option<u32> {
        match pwrzv::get_power_reserve_level_direct().await {
            Ok(level) => {
                let clamped = Self::clamp_power_reserve(level);
                debug!("Power reserve level (init): {:.2} -> {}", level, clamped);
                Some(clamped)
            }
            Err(e) => {
                warn!(
                    "Failed to get power reserve level from pwrzv (init): {}, returning None",
                    e
                );
                None
            }
        }
    }

    fn clamp_power_reserve(level: f32) -> u32 {
        level.clamp(0.0, 5.0).round() as u32
    }

    /// 断开连接
    pub fn disconnect(&mut self) {
        self.client = None;
        info!("Disconnected from supervisor");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actrix_common::{ServiceInfo, ServiceState, ServiceType};

    #[test]
    fn test_client_creation() {
        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            shared_secret: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            ..Default::default()
        };

        let service_collector = ServiceCollector::new();
        let client = SupervitClient::new(config, service_collector);
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_build_service_advertisements_merges_tags() {
        use actrix_proto::ResourceType;

        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            shared_secret: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            service_tags: vec!["beta".to_string(), "alpha".to_string()],
            ..Default::default()
        };

        let service_info = ServiceInfo {
            name: "turn-service".to_string(),
            service_type: ServiceType::Turn,
            domain_name: "turn:example.com".to_string(),
            port_info: "3478".to_string(),
            status: ServiceState::Running("turn:example.com:3478".to_string()),
            description: None,
        };
        let registry = ServiceCollector::new();
        registry.insert("turn".to_string(), service_info).await;
        let service_collector = registry;

        let client = SupervitClient::new(config, service_collector).unwrap();
        let services = client.build_service_advertisements().await;

        assert_eq!(services.len(), 1);
        assert_eq!(
            services[0].tags,
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn test_client_invalid_config() {
        let config = SupervitConfig {
            node_id: String::new(),
            ..Default::default()
        };

        let service_collector = ServiceCollector::new();
        let client = SupervitClient::new(config, service_collector);
        assert!(client.is_err());
    }

    #[tokio::test]
    async fn test_create_report_request() {
        let secret =
            hex::decode("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let service_collector = ServiceCollector::new();
        let report = SupervitClient::create_report_request(
            "test-node",
            "test-location",
            "test-name",
            &secret,
            service_collector,
        )
        .await;
        assert!(report.is_ok());
        let report = report.unwrap();
        assert_eq!(report.node_id, "test-node"); // proto field name unchanged
        assert_eq!(report.location_tag, "test-location");
        assert_eq!(report.name, "test-name");
        // proto2 required 字段不是 Option
        assert!(report.credential.timestamp > 0);
        assert!(!report.credential.nonce.is_empty());
        assert!(!report.credential.signature.is_empty());
    }

    #[test]
    fn test_tls_config_validation() {
        // 测试：启用 TLS 但未提供 tls_domain 应该失败
        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "https://localhost:50051".to_string(),
            shared_secret: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            enable_tls: true,
            tls_domain: None, // 缺少 tls_domain
            ..Default::default()
        };

        let service_collector = ServiceCollector::new();
        let result = SupervitClient::new(config, service_collector);
        assert!(result.is_err());
    }

    #[test]
    fn test_tls_config_with_domain() {
        // 测试：启用 TLS 并提供 tls_domain 应该成功
        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "https://localhost:50051".to_string(),
            shared_secret: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            enable_tls: true,
            tls_domain: Some("localhost".to_string()),
            ..Default::default()
        };

        let service_collector = ServiceCollector::new();
        let client = SupervitClient::new(config, service_collector);
        assert!(client.is_ok());
    }

    #[test]
    fn test_mtls_partial_config_error() {
        // 测试：只配置客户端证书但不配置私钥应该在验证阶段失败
        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "https://localhost:50051".to_string(),
            shared_secret: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            enable_tls: true,
            tls_domain: Some("localhost".to_string()),
            client_cert: Some("/path/to/cert.pem".to_string()),
            client_key: None, // 缺少私钥
            ..Default::default()
        };

        // validate() 会检查 mTLS 配置完整性，应该失败
        let service_collector = ServiceCollector::new();
        let client = SupervitClient::new(config, service_collector);
        assert!(client.is_err());
    }

    #[test]
    fn test_tls_disabled() {
        // 测试：不启用 TLS 应该可以正常创建客户端
        let config = SupervitConfig {
            node_id: "test-node".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            shared_secret: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            ),
            enable_tls: false,
            ..Default::default()
        };

        let service_collector = ServiceCollector::new();
        let client = SupervitClient::new(config, service_collector);
        assert!(client.is_ok());
    }
}
