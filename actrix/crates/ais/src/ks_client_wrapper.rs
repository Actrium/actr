//! KS 客户端包装器
//!
//! 提供统一的 KS 客户端接口，支持 gRPC 客户端（需要 &mut self）

use actrix_common::aid::AidError;
use ecies::{PublicKey, SecretKey};
use ks::GrpcClient;
use std::sync::Arc;
use tokio::sync::RwLock;

/// KS 客户端包装器（用于 gRPC 客户端）
#[derive(Clone)]
pub struct KsClientWrapper {
    inner: Arc<RwLock<GrpcClient>>,
}

impl KsClientWrapper {
    /// 创建新的 KS 客户端包装器
    pub fn new(client: GrpcClient) -> Self {
        Self {
            inner: Arc::new(RwLock::new(client)),
        }
    }

    /// 生成密钥对
    pub async fn generate_key(&self) -> Result<(u32, PublicKey, u64, u64), ks::KsError> {
        let mut client = self.inner.write().await;
        client.generate_key().await
    }

    /// 获取私钥
    pub async fn fetch_secret_key(
        &self,
        key_id: u32,
    ) -> Result<(SecretKey, u64, bool), ks::KsError> {
        let mut client = self.inner.write().await;
        client.fetch_secret_key(key_id).await
    }

    /// 健康检查
    pub async fn health_check(&self) -> Result<String, ks::KsError> {
        let mut client = self.inner.write().await;
        client.health_check().await
    }
}

/// 从配置创建 KS 客户端包装器
pub async fn create_ks_client(
    config: &actrix_common::config::ks::KsClientConfig,
    actrix_shared_key: &str,
) -> Result<KsClientWrapper, AidError> {
    let grpc_config = ks::GrpcClientConfig {
        endpoint: config.endpoint.clone(),
        actrix_shared_key: actrix_shared_key.to_string(),
        timeout_seconds: config.timeout_seconds,
        enable_tls: config.enable_tls,
        tls_domain: config.tls_domain.clone(),
        ca_cert: config.ca_cert.clone(),
        client_cert: config.client_cert.clone(),
        client_key: config.client_key.clone(),
    };

    let client = GrpcClient::new(&grpc_config)
        .await
        .map_err(|e| AidError::GenerationFailed(format!("Failed to create KS client: {e}")))?;

    Ok(KsClientWrapper::new(client))
}
