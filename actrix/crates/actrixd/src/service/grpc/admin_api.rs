use actrix_sdk::control::{AdminApiService, AuthService, NodeAdminServiceServer};
use anyhow::Result;
use platform::{ServiceCollector, config::AdminPlaneConfig, storage::nonce::SqliteNonceStorage};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tonic::transport::Server;

/// Admin API gRPC service launcher.
///
/// 当前实现承载的是 Admin 回连节点时使用的 NodeAdminService。
#[derive(Debug)]
pub struct AdminApiGrpcService {
    admin_config: AdminPlaneConfig,
    sqlite_path: PathBuf,
    location_tag: String,
    service_collector: ServiceCollector,
}

impl AdminApiGrpcService {
    /// Create new Admin API gRPC service launcher
    ///
    /// - `admin_config`: validated admin configuration
    /// - `sqlite_path`: base directory for SQLite databases (used for nonce.db)
    /// - `location_tag`: node location tag reported to admin
    /// - `service_collector`: service collector for accessing service statuses
    pub fn new(
        admin_config: AdminPlaneConfig,
        sqlite_path: PathBuf,
        location_tag: String,
        service_collector: ServiceCollector,
    ) -> Self {
        Self {
            admin_config,
            sqlite_path,
            location_tag,
            service_collector,
        }
    }

    /// Start Admin API gRPC service
    pub async fn start(
        &mut self,
        addr: SocketAddr,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Result<JoinHandle<()>> {
        let admin_cfg = &self.admin_config;

        let client_cfg = &admin_cfg.client;
        let shared_secret = Arc::new(
            hex::decode(admin_cfg.shared_secret())
                .map_err(|e| anyhow::anyhow!("Invalid shared_secret hex for admin_api: {e}"))?,
        );

        let node_id = client_cfg.node_id.clone();
        let node_name = admin_cfg.node_name().to_string();

        // Initialize nonce storage (anti-replay)
        let nonce_storage = Arc::new(
            SqliteNonceStorage::new_async(&self.sqlite_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to init nonce storage: {e}"))?,
        );

        // Build admin_api service instance
        // ServiceCollector now uses ServiceInfo internally, so we can pass it directly
        let mut service = AdminApiService::new(
            node_id.clone(),
            node_name,
            self.location_tag.clone(),
            env!("CARGO_PKG_VERSION"),
            self.service_collector.clone(),
        )
        .map_err(|e| anyhow::anyhow!("Failed to create admin_api service: {e}"))?;

        // Shutdown handling: broadcast shutdown signal
        let shutdown_tx_for_handler = shutdown_tx.clone();
        service = service.with_shutdown_handler(move |_graceful, _timeout, reason| {
            let shutdown_tx = shutdown_tx_for_handler.clone();
            async move {
                if let Some(reason) = reason {
                    platform::recording::warn!("AdminApi shutdown requested: {}", reason);
                } else {
                    platform::recording::warn!("AdminApi shutdown requested");
                }
                let _ = shutdown_tx.send(());
                Ok(())
            }
        });

        platform::recording::info!("🚀 Starting AdminApi gRPC service on {}", addr);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let max_clock_skew_secs = admin_cfg.max_clock_skew_secs;
        let handle = tokio::spawn(async move {
            let authed_service = AuthService::new(
                service,
                node_id,
                shared_secret,
                nonce_storage,
                max_clock_skew_secs,
            );
            let result = Server::builder()
                .add_service(NodeAdminServiceServer::new(authed_service))
                .serve_with_shutdown(addr, async move {
                    platform::recording::info!("✅ AdminApi gRPC service listening on {}", addr);
                    let _ = shutdown_rx.recv().await;
                    platform::recording::info!("AdminApi gRPC service received shutdown signal");
                })
                .await;

            if let Err(err) = result {
                platform::recording::error!("AdminApi gRPC service error: {}", err);
            }

            let _ = shutdown_tx.send(());
        });

        Ok(handle)
    }
}
