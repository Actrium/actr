//! STUN服务实现

use crate::service::IceService;
use anyhow::Result;
use async_trait::async_trait;
use platform::config::ActrixConfig;
use platform::status::services::ServiceState;
use platform::{ServiceInfo, ServiceType};
use std::sync::Arc;
use stun;
use tokio::net::UdpSocket;
use url::Url;

/// STUN服务实现
#[derive(Debug)]
pub struct StunService {
    info: ServiceInfo,
    config: ActrixConfig,
    socket: Option<Arc<UdpSocket>>,
}

impl StunService {
    pub fn new(config: ActrixConfig) -> Self {
        Self {
            info: ServiceInfo::new(
                "STUN Server",
                ServiceType::Stun,
                Some("STUN server for WebRTC connectivity".to_string()),
                &config,
            ),
            config,
            socket: None,
        }
    }
}

#[async_trait]
impl IceService for StunService {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn info_mut(&mut self) -> &mut ServiceInfo {
        &mut self.info
    }

    async fn start(
        &mut self,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
        oneshot_tx: tokio::sync::oneshot::Sender<ServiceInfo>,
    ) -> Result<()> {
        let ice_bind = &self.config.bind.ice;
        let addr = format!("{}:{}", ice_bind.ip, ice_bind.port);

        platform::recording::info!("Starting STUN service on {}", addr);

        // 绑定UDP套接字
        let socket = match UdpSocket::bind(&addr).await {
            Ok(socket) => {
                platform::recording::info!("STUN service listening on: {}", addr);
                Arc::new(socket)
            }
            Err(e) => {
                let error_msg = format!("Failed to bind STUN service to {addr}: {e}");
                self.info.set_error(&error_msg);
                return Err(anyhow::anyhow!(error_msg));
            }
        };

        self.socket = Some(socket.clone());

        // 设置运行状态
        let url = Url::parse(&format!("stun:{}:{}", ice_bind.domain_name, ice_bind.port))?;
        self.info.set_running(url);
        oneshot_tx
            .send(self.info.clone())
            .map_err(|e| anyhow::anyhow!("Failed to send STUN service info: {e:?}"))?;
        platform::recording::info!("STUN service started successfully");

        // 启动STUN服务器（带优雅关闭支持）
        if let Err(e) = stun::create_stun_server_with_shutdown(socket.clone(), shutdown_rx).await {
            let error_msg = format!("STUN server stopped with error: {e}");
            self.info.set_error(&error_msg);
            platform::recording::error!("{}", error_msg);
        } else {
            platform::recording::info!("STUN server shut down gracefully");
        }

        self.stop().await?;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        platform::recording::info!("Stopping STUN service");

        // 清理状态
        self.socket = None;
        self.info.status = ServiceState::Unknown;

        platform::recording::info!("STUN service stopped");
        Ok(())
    }
}
