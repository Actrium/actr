//! KS gRPC 客户端

use crate::error::KsError;
use actrix_proto::ks::v1::{
    GenerateKeyRequest, GetSecretKeyRequest, HealthCheckRequest, key_server_client::KeyServerClient,
};
use actrix_proto::supervisor::v1::NonceCredential;
use base64::prelude::*;
use ecies::{PublicKey, SecretKey};
use nonce_auth::CredentialBuilder;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tracing::{debug, info, warn};

/// KS gRPC 客户端配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GrpcClientConfig {
    /// KS 服务地址 (gRPC endpoint)
    ///
    /// 例如: "http://127.0.0.1:50052" 或 "https://ks.example.com:50052"
    pub endpoint: String,

    /// Actrix 共享密钥（用于认证）
    pub actrix_shared_key: String,

    /// 请求超时时间（秒）
    pub timeout_seconds: u64,

    /// 是否启用 TLS
    pub enable_tls: bool,

    /// TLS 域名（启用 TLS 时必需）
    pub tls_domain: Option<String>,

    /// CA 证书路径（用于验证服务端）
    pub ca_cert: Option<String>,

    /// 客户端证书路径（mTLS）
    pub client_cert: Option<String>,

    /// 客户端私钥路径（mTLS）
    pub client_key: Option<String>,
}

/// KS gRPC 客户端
pub struct GrpcClient {
    client: KeyServerClient<Channel>,
    actrix_shared_key: String,
}

impl GrpcClient {
    /// 创建新的 KS gRPC 客户端
    pub async fn new(config: &GrpcClientConfig) -> Result<Self, KsError> {
        let mut endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|e| KsError::Internal(format!("Invalid endpoint: {e}")))?
            .timeout(Duration::from_secs(config.timeout_seconds))
            .connect_timeout(Duration::from_secs(config.timeout_seconds));

        // 如果启用 TLS，配置 TLS/mTLS
        if config.enable_tls {
            let tls_config = Self::build_tls_config(config)?;
            endpoint = endpoint
                .tls_config(tls_config)
                .map_err(|e| KsError::Internal(format!("TLS configuration error: {e}")))?;
            info!("TLS enabled for KS gRPC client");
        }

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| KsError::Internal(format!("Failed to connect to KS: {e}")))?;

        let client = KeyServerClient::new(channel);

        Ok(Self {
            client,
            actrix_shared_key: config.actrix_shared_key.clone(),
        })
    }

    /// 构建 TLS 配置
    fn build_tls_config(config: &GrpcClientConfig) -> Result<ClientTlsConfig, KsError> {
        let tls_domain = config.tls_domain.as_ref().ok_or_else(|| {
            KsError::Config("tls_domain is required when enable_tls is true".to_string())
        })?;

        let mut tls_config = ClientTlsConfig::new().domain_name(tls_domain);

        debug!("Configuring TLS with domain: {}", tls_domain);

        // 加载 CA 证书
        if let Some(ca_cert_path) = &config.ca_cert {
            debug!("Loading CA certificate from: {}", ca_cert_path);
            let ca_cert_pem = std::fs::read(ca_cert_path).map_err(|e| {
                KsError::Config(format!(
                    "Failed to read CA certificate from {ca_cert_path}: {e}"
                ))
            })?;

            let ca_cert = Certificate::from_pem(ca_cert_pem);
            tls_config = tls_config.ca_certificate(ca_cert);
            info!("CA certificate loaded for server verification");
        }

        // 加载客户端证书和私钥（mTLS）
        if let (Some(cert_path), Some(key_path)) = (&config.client_cert, &config.client_key) {
            debug!("Loading client certificate from: {}", cert_path);
            debug!("Loading client private key from: {}", key_path);

            let client_cert_pem = std::fs::read(cert_path).map_err(|e| {
                KsError::Config(format!(
                    "Failed to read client certificate from {cert_path}: {e}"
                ))
            })?;

            let client_key_pem = std::fs::read(key_path).map_err(|e| {
                KsError::Config(format!(
                    "Failed to read client private key from {key_path}: {e}"
                ))
            })?;

            let identity = Identity::from_pem(client_cert_pem, client_key_pem);
            tls_config = tls_config.identity(identity);
            info!("mTLS enabled: client certificate and key loaded");
        } else if config.client_cert.is_some() || config.client_key.is_some() {
            return Err(KsError::Config(
                "Both client_cert and client_key must be provided for mTLS".to_string(),
            ));
        }

        Ok(tls_config)
    }

    /// 从 KS 服务生成新的密钥对
    pub async fn generate_key(&mut self) -> Result<(u32, PublicKey, u64, u64), KsError> {
        let request_data = "generate_key";

        // 创建 nonce credential
        let nonce_credential = CredentialBuilder::new(self.actrix_shared_key.as_bytes())
            .sign(request_data.as_bytes())?;

        // 转换为 protobuf NonceCredential
        let credential = NonceCredential {
            timestamp: nonce_credential.timestamp,
            nonce: nonce_credential.nonce,
            signature: nonce_credential.signature,
        };

        let request = tonic::Request::new(GenerateKeyRequest { credential });

        debug!("Requesting key generation from KS via gRPC");

        let response = self
            .client
            .generate_key(request)
            .await
            .map_err(|e| KsError::Internal(format!("gRPC GenerateKey failed: {e}")))?;

        let resp = response.into_inner();

        // 解码公钥
        let public_key_bytes = BASE64_STANDARD
            .decode(&resp.public_key)
            .map_err(|e| KsError::Crypto(format!("Failed to decode public key: {e}")))?;

        if public_key_bytes.len() == 33 {
            let public_key_array: [u8; 33] = public_key_bytes
                .try_into()
                .map_err(|_| KsError::Crypto("Invalid public key length".to_string()))?;
            let public_key = PublicKey::parse_compressed(&public_key_array).map_err(|e| {
                KsError::Crypto(format!("Failed to parse compressed public key: {e}"))
            })?;

            info!(
                "Successfully generated key pair with key_id {} via gRPC, expires_at: {}, tolerance_seconds: {}",
                resp.key_id, resp.expires_at, resp.tolerance_seconds
            );
            Ok((
                resp.key_id,
                public_key,
                resp.expires_at,
                resp.tolerance_seconds,
            ))
        } else {
            Err(KsError::Crypto(format!(
                "Unsupported public key length: {}",
                public_key_bytes.len()
            )))
        }
    }

    /// 从 KS 服务获取私钥、过期时间和容忍期状态
    ///
    /// 返回 (SecretKey, expires_at, in_tolerance_period)
    pub async fn fetch_secret_key(
        &mut self,
        key_id: u32,
    ) -> Result<(SecretKey, u64, bool), KsError> {
        let request_data = format!("get_secret_key:{key_id}");

        // 创建 nonce credential
        let nonce_credential = CredentialBuilder::new(self.actrix_shared_key.as_bytes())
            .sign(request_data.as_bytes())?;

        // 转换为 protobuf NonceCredential
        let credential = NonceCredential {
            timestamp: nonce_credential.timestamp,
            nonce: nonce_credential.nonce,
            signature: nonce_credential.signature,
        };

        let request = tonic::Request::new(GetSecretKeyRequest { key_id, credential });

        debug!("Fetching secret key {} from KS via gRPC", key_id);

        let response = self
            .client
            .get_secret_key(request)
            .await
            .map_err(|e| KsError::Internal(format!("gRPC GetSecretKey failed: {e}")))?;

        let resp = response.into_inner();

        // 解码私钥
        let secret_key_bytes = BASE64_STANDARD
            .decode(&resp.secret_key)
            .map_err(|e| KsError::Crypto(format!("Failed to decode secret key: {e}")))?;

        let secret_key_array: [u8; 32] = secret_key_bytes.try_into().map_err(|_| {
            KsError::Crypto("Invalid secret key length, expected 32 bytes".to_string())
        })?;

        let secret_key = SecretKey::parse(&secret_key_array)
            .map_err(|e| KsError::Crypto(format!("Failed to parse secret key: {e}")))?;

        if resp.in_tolerance_period {
            warn!(
                "Key {} is in tolerance period (expired but still valid)",
                key_id
            );
        }

        info!(
            "Successfully fetched secret key {} from KS via gRPC, expires_at: {}, in_tolerance: {}",
            key_id, resp.expires_at, resp.in_tolerance_period
        );
        Ok((secret_key, resp.expires_at, resp.in_tolerance_period))
    }

    /// 健康检查
    pub async fn health_check(&mut self) -> Result<String, KsError> {
        let request = tonic::Request::new(HealthCheckRequest {});

        let response = self
            .client
            .health_check(request)
            .await
            .map_err(|e| KsError::Internal(format!("gRPC HealthCheck failed: {e}")))?;

        let resp = response.into_inner();
        Ok(resp.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_client_config() {
        let config = GrpcClientConfig {
            endpoint: "http://127.0.0.1:50052".to_string(),
            actrix_shared_key: "test-key".to_string(),
            timeout_seconds: 30,
            enable_tls: false,
            tls_domain: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
        };

        assert_eq!(config.endpoint, "http://127.0.0.1:50052");
        assert!(!config.enable_tls);
    }

    #[test]
    fn test_tls_config_validation() {
        let config = GrpcClientConfig {
            endpoint: "https://ks.example.com:50052".to_string(),
            actrix_shared_key: "test-key".to_string(),
            timeout_seconds: 30,
            enable_tls: true,
            tls_domain: None, // 缺少 tls_domain
            ca_cert: None,
            client_cert: None,
            client_key: None,
        };

        let result = GrpcClient::build_tls_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_mtls_partial_config() {
        let config = GrpcClientConfig {
            endpoint: "https://ks.example.com:50052".to_string(),
            actrix_shared_key: "test-key".to_string(),
            timeout_seconds: 30,
            enable_tls: true,
            tls_domain: Some("ks.example.com".to_string()),
            ca_cert: None,
            client_cert: Some("/path/to/cert.pem".to_string()),
            client_key: None, // 缺少 client_key
        };

        let result = GrpcClient::build_tls_config(&config);
        assert!(result.is_err());
    }
}
