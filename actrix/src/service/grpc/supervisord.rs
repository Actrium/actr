use actrix_common::{
    ServiceCollector, config::SupervisorConfig, storage::nonce::SqliteNonceStorage,
};
use anyhow::Result;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use supervit::{AuthService, SupervisedServiceServer, Supervisord};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tonic::transport::Server;
use tracing::{error, info, warn};

/// Supervisord gRPC service launcher
#[derive(Debug)]
pub struct SupervisordGrpcService {
    supervisor_config: SupervisorConfig,
    sqlite_path: PathBuf,
    location_tag: String,
    service_collector: ServiceCollector,
}

impl SupervisordGrpcService {
    /// Create new Supervisord gRPC service launcher
    ///
    /// - `supervisor_config`: validated supervisor configuration
    /// - `sqlite_path`: base directory for SQLite databases (used for nonce.db)
    /// - `location_tag`: node location tag reported to supervisor
    /// - `service_collector`: service collector for accessing service statuses
    pub fn new(
        supervisor_config: SupervisorConfig,
        sqlite_path: PathBuf,
        location_tag: String,
        service_collector: ServiceCollector,
    ) -> Self {
        Self {
            supervisor_config,
            sqlite_path,
            location_tag,
            service_collector,
        }
    }

    /// Start Supervisord gRPC service
    pub async fn start(
        &mut self,
        addr: SocketAddr,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Result<JoinHandle<()>> {
        let supervisor_cfg = &self.supervisor_config;

        let client_cfg = &supervisor_cfg.client;
        let shared_secret = Arc::new(
            hex::decode(supervisor_cfg.shared_secret())
                .map_err(|e| anyhow::anyhow!("Invalid shared_secret hex for supervisord: {e}"))?,
        );

        let node_id = client_cfg.node_id.clone();
        let node_name = supervisor_cfg.node_name().to_string();

        // Initialize nonce storage (anti-replay)
        let nonce_storage = Arc::new(
            SqliteNonceStorage::new_async(&self.sqlite_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to init nonce storage: {e}"))?,
        );

        // Build supervisord service instance
        // ServiceCollector now uses ServiceInfo internally, so we can pass it directly
        let mut service = Supervisord::new(
            node_id.clone(),
            node_name,
            self.location_tag.clone(),
            env!("CARGO_PKG_VERSION"),
            self.service_collector.clone(),
        )
        .map_err(|e| anyhow::anyhow!("Failed to create supervisord service: {e}"))?;

        // Shutdown handling: broadcast shutdown signal
        let shutdown_tx_for_handler = shutdown_tx.clone();
        service = service.with_shutdown_handler(move |_graceful, _timeout, reason| {
            let shutdown_tx = shutdown_tx_for_handler.clone();
            async move {
                if let Some(reason) = reason {
                    warn!("Supervisord shutdown requested: {}", reason);
                } else {
                    warn!("Supervisord shutdown requested");
                }
                let _ = shutdown_tx.send(());
                Ok(())
            }
        });

        info!("🚀 Starting Supervisord gRPC service on {}", addr);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let max_clock_skew_secs = supervisor_cfg.max_clock_skew_secs;
        let handle = tokio::spawn(async move {
            let authed_service = AuthService::new(
                service,
                node_id,
                shared_secret,
                nonce_storage,
                max_clock_skew_secs,
            );
            let result = Server::builder()
                .add_service(SupervisedServiceServer::new(authed_service))
                .serve_with_shutdown(addr, async move {
                    info!("✅ Supervisord gRPC service listening on {}", addr);
                    let _ = shutdown_rx.recv().await;
                    info!("Supervisord gRPC service received shutdown signal");
                })
                .await;

            if let Err(err) = result {
                error!("Supervisord gRPC service error: {}", err);
            }

            let _ = shutdown_tx.send(());
        });

        Ok(handle)
    }
}
