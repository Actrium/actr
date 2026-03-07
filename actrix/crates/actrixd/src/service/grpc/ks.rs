//! KS (Key Server) gRPC 路由构建
//!
//! 将 KS gRPC 服务挂载到主 HTTP/HTTPS 监听端口。

use anyhow::Result;
use axum::Router;
use ks::{KeyEncryptor, KeyStorage, create_grpc_service};
use platform::{
    config::ActrixConfig, monitoring::ServiceCounters, storage::nonce::SqliteNonceStorage,
};
use std::sync::Arc;

/// Build KS gRPC router mounted on the main HTTP listener.
///
/// Primary route for tonic clients:
/// `/ks.v1.KeyServer/<Method>`
///
/// When `counters` is provided, an axum middleware layer records every
/// request into the shared `ServiceCounters`.
pub async fn build_ks_grpc_router(
    config: &ActrixConfig,
    counters: Option<Arc<ServiceCounters>>,
) -> Result<Router> {
    let ks_service_config = config
        .services
        .ks
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("KS service configuration not found"))?;

    // 创建 nonce storage 实例（用于防重放攻击）
    // 使用 sqlite_path 作为目录路径，内部会自动拼接 nonce.db
    let nonce_storage = SqliteNonceStorage::new_async(&config.sqlite_path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create nonce storage: {e}"))?;

    // 创建密钥加密器
    let encryptor = match ks_service_config.get_kek_source() {
        Some(kek_source) => {
            platform::recording::info!("KEK configured, enabling private key encryption");
            KeyEncryptor::from_kek_source(&kek_source)
                .map_err(|e| anyhow::anyhow!("Failed to create key encryptor: {e}"))?
        }
        None => {
            platform::recording::info!(
                "No KEK configured, private keys will be stored in plaintext"
            );
            KeyEncryptor::no_encryption()
        }
    };

    // 创建 KS storage
    let storage =
        KeyStorage::from_config(&ks_service_config.storage, encryptor, &config.sqlite_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create KS storage: {e}"))?;

    // 创建 gRPC 服务
    let grpc_service = create_grpc_service(
        storage,
        nonce_storage,
        config.actrix_shared_key.clone(),
        ks_service_config.tolerance_seconds,
    );

    platform::recording::info!("KS gRPC service mounted on primary HTTP listener");

    let mut router = Router::new().route_service("/ks.v1.KeyServer/{*grpc_method}", grpc_service);

    // Wrap with a metrics middleware when counters are provided
    if let Some(ctr) = counters {
        router = router.layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let ctr = ctr.clone();
                async move {
                    let start = std::time::Instant::now();
                    let response = next.run(req).await;
                    let success = response.status().is_success();
                    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
                    ctr.record_request(success, latency_ms).await;
                    response
                }
            },
        ));
    }

    Ok(router)
}
