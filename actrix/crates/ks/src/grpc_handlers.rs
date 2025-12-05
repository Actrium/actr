//! KS gRPC 服务实现

use crate::{error::KsError, storage::KeyStorage};
use nonce_auth::{CredentialVerifier, NonceError, storage::NonceStorage};
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

// 导入生成的 protobuf 代码
use actrix_proto::ks::v1::key_server_server::{KeyServer, KeyServerServer};
use actrix_proto::ks::v1::*;
use actrix_proto::supervisor::v1::NonceCredential;

/// KS gRPC 服务状态
#[derive(Clone)]
pub struct KsGrpcService {
    pub storage: KeyStorage,
    pub nonce_storage: Arc<dyn NonceStorage + Send + Sync>,
    pub psk: String,
}

impl KsGrpcService {
    /// 创建新的 gRPC 服务实例
    pub fn new<N: NonceStorage + Send + Sync + 'static>(
        storage: KeyStorage,
        nonce_storage: N,
        psk: String,
    ) -> Self {
        Self {
            storage,
            nonce_storage: Arc::new(nonce_storage),
            psk,
        }
    }

    /// 验证请求的 nonce 凭证
    async fn verify_credential(
        &self,
        credential: &NonceCredential,
        request_payload: &str,
    ) -> Result<(), KsError> {
        // 将 protobuf NonceCredential 转换为 nonce_auth::NonceCredential
        let nonce_credential = nonce_auth::NonceCredential {
            timestamp: credential.timestamp,
            nonce: credential.nonce.clone(),
            signature: credential.signature.clone(),
        };

        let verify_result = CredentialVerifier::new(self.nonce_storage.clone())
            .with_secret(self.psk.as_bytes())
            .verify(&nonce_credential, request_payload.as_bytes())
            .await;

        verify_result.map_err(|e| match e {
            NonceError::DuplicateNonce => KsError::ReplayAttack("Nonce already used".to_string()),
            NonceError::TimestampOutOfWindow => {
                KsError::Authentication("Request timestamp out of range".to_string())
            }
            NonceError::InvalidSignature => {
                KsError::Authentication("Invalid signature".to_string())
            }
            _ => KsError::Internal(format!("Authentication error: {e}")),
        })?;

        Ok(())
    }
}

#[tonic::async_trait]
impl KeyServer for KsGrpcService {
    /// 生成新的密钥对
    async fn generate_key(
        &self,
        request: Request<GenerateKeyRequest>,
    ) -> Result<Response<GenerateKeyResponse>, Status> {
        info!("Received gRPC GenerateKey request");

        let req = request.into_inner();

        // 验证凭证（proto2 required 字段直接是结构体类型）
        let request_data = "generate_key";
        self.verify_credential(&req.credential, request_data)
            .await
            .map_err(|e| Status::unauthenticated(format!("Authentication failed: {e}")))?;

        // 生成密钥对
        let key_pair = self
            .storage
            .generate_and_store_key()
            .await
            .map_err(|e| Status::internal(format!("Failed to generate key: {e}")))?;

        // 获取密钥记录以获取过期时间
        let key_record = self
            .storage
            .get_key_record(key_pair.key_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get key record: {e}")))?
            .ok_or_else(|| Status::internal("Failed to get key record after creation"))?;

        info!("Generated key pair with key_id: {}", key_pair.key_id);

        let response = GenerateKeyResponse {
            key_id: key_pair.key_id,
            public_key: key_pair.public_key,
            expires_at: key_record.expires_at,
        };

        Ok(Response::new(response))
    }

    /// 获取指定 key_id 的私钥
    async fn get_secret_key(
        &self,
        request: Request<GetSecretKeyRequest>,
    ) -> Result<Response<GetSecretKeyResponse>, Status> {
        let req = request.into_inner();
        let key_id = req.key_id;

        info!("Received gRPC GetSecretKey request for key_id: {}", key_id);

        // 验证凭证（proto2 required 字段直接是结构体类型）
        let request_data = format!("get_secret_key:{key_id}");
        self.verify_credential(&req.credential, &request_data)
            .await
            .map_err(|e| Status::unauthenticated(format!("Authentication failed: {e}")))?;

        // 获取完整的密钥记录
        let key_record = self
            .storage
            .get_key_record(key_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get key record: {e}")))?
            .ok_or_else(|| Status::not_found(format!("Key not found: {key_id}")))?;

        // 检查密钥是否过期
        if key_record.expires_at > 0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if key_record.expires_at < now {
                warn!("Key {} has expired", key_id);
                return Err(Status::not_found(format!("Key {key_id} has expired")));
            }
        }

        // 获取私钥
        let secret_key = self
            .storage
            .get_secret_key(key_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get secret key: {e}")))?
            .ok_or_else(|| Status::not_found(format!("Secret key not found: {key_id}")))?;

        info!("Found secret key for key_id: {}", key_id);

        let response = GetSecretKeyResponse {
            key_id,
            secret_key,
            expires_at: key_record.expires_at,
        };

        Ok(Response::new(response))
    }

    /// 健康检查
    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        debug!("gRPC health check requested");

        let key_count = self
            .storage
            .get_key_count()
            .await
            .map_err(|e| Status::internal(format!("Failed to get key count: {e}")))?;

        let response = HealthCheckResponse {
            status: "healthy".to_string(),
            service: "ks".to_string(),
            backend: self.storage.backend_name().to_string(),
            key_count,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        Ok(Response::new(response))
    }
}

/// 创建 gRPC 服务器
pub fn create_grpc_service<N: NonceStorage + Send + Sync + 'static>(
    storage: KeyStorage,
    nonce_storage: N,
    psk: String,
) -> KeyServerServer<KsGrpcService> {
    let service = KsGrpcService::new(storage, nonce_storage, psk);
    KeyServerServer::new(service)
}
