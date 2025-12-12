//! KS (Key Server) gRPC 服务实现
//!
//! 提供椭圆曲线密钥生成和管理的 gRPC API 服务

use actrix_common::{config::ActrixConfig, storage::nonce::SqliteNonceStorage};
use anyhow::Result;
use ks::{KeyEncryptor, KeyStorage, create_grpc_service};
use std::net::SocketAddr;
use tokio::{sync::broadcast, task::JoinHandle};
use tonic::transport::Server;
use tracing::{error, info};

/// KS gRPC 服务实现
#[derive(Debug)]
pub struct KsGrpcService {
    config: ActrixConfig,
}

impl KsGrpcService {
    pub fn new(config: ActrixConfig) -> Self {
        Self { config }
    }

    /// 启动 gRPC 服务器
    pub async fn start(
        &mut self,
        addr: SocketAddr,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Result<JoinHandle<()>> {
        info!("Starting KS gRPC service on {}", addr);

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

        // 创建密钥加密器
        let encryptor = match ks_service_config.get_kek_source() {
            Some(kek_source) => {
                info!("KEK configured, enabling private key encryption");
                KeyEncryptor::from_kek_source(&kek_source)
                    .map_err(|e| anyhow::anyhow!("Failed to create key encryptor: {e}"))?
            }
            None => {
                info!("No KEK configured, private keys will be stored in plaintext");
                KeyEncryptor::no_encryption()
            }
        };

        // 创建 KS storage
        let storage = KeyStorage::from_config(
            &ks_service_config.storage,
            encryptor,
            &self.config.sqlite_path,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create KS storage: {e}"))?;

        // 创建 gRPC 服务
        let grpc_service = create_grpc_service(
            storage,
            nonce_storage,
            self.config.actrix_shared_key.clone(),
            ks_service_config.tolerance_seconds,
        );

        info!("KS gRPC service created successfully");

        let mut shutdown_rx = shutdown_tx.subscribe();
        let handle = tokio::spawn(async move {
            Server::builder()
                .add_service(grpc_service)
                .serve_with_shutdown(addr, async move {
                    let _ = shutdown_rx.recv().await;
                    info!("KS gRPC service received shutdown signal");
                })
                .await
                .map_err(|err| error!("KS gRPC service error: {}", err))
                .ok();
            let _ = shutdown_tx.send(());
        });

        info!("✅ KS gRPC service listening on {}", addr);

        Ok(handle)
    }
}
