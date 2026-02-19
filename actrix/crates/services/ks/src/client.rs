//! KS 客户端 - 简单的 HTTP 客户端

use crate::types::{GenerateKeyRequest, GenerateKeyResponse, GetSecretKeyResponse};
use base64::prelude::*;
use ecies::{PublicKey, SecretKey};
use nonce_auth::CredentialBuilder;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

/// KS 服务客户端
#[derive(Debug, Clone)]
pub struct Client {
    endpoint: String,
    client: reqwest::Client,
    actrix_shared_key: String,
}

/// KS 客户端配置
///
/// 其他服务作为客户端连接 KS 服务时使用的配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClientConfig {
    /// KS 服务地址
    ///
    /// KS 服务的完整 URL 地址，包括协议、主机和端口。
    /// 例如: "http://127.0.0.1:8090" 或 "https://ks.example.com"
    pub endpoint: String,

    /// PSK (Pre-Shared Key) - 用于认证
    ///
    /// 用于内部服务间的认证密钥，通常从全局配置的 `actrix_shared_key` 获取
    pub psk: String,

    /// 请求超时时间（秒）
    ///
    /// 连接 KS 服务的超时时间
    pub timeout_seconds: u64,
}

impl Client {
    /// 创建新的 KS 客户端
    pub fn new(config: &ClientConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            endpoint: config.endpoint.clone(),
            client,
            actrix_shared_key: config.psk.clone(),
        }
    }

    /// 从 KS 服务生成新的密钥对
    pub async fn generate_key(&self) -> Result<(u32, PublicKey, u64), crate::error::KsError> {
        let url = format!("{}/generate", self.endpoint);

        // 构建请求数据用于签名
        let request_data = "generate_key";

        // 创建 nonce credential
        let credential = CredentialBuilder::new(self.actrix_shared_key.as_bytes())
            .sign(request_data.as_bytes())?;

        let request = GenerateKeyRequest { credential };

        debug!("Requesting key generation from KS at {}", url);

        // 发送请求
        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::error::KsError::Internal(format!(
                "KS generate key request failed with status {status}: {error_text}"
            )));
        }

        // 解析响应
        let response: GenerateKeyResponse = response.json().await?;

        // 解码公钥
        let public_key_bytes = BASE64_STANDARD.decode(&response.public_key)?;

        // PublicKey::parse 需要 &[u8; 33] 或 &[u8; 65] 类型
        if public_key_bytes.len() == 33 {
            let public_key_array: [u8; 33] = public_key_bytes.try_into().map_err(|_| {
                crate::error::KsError::Crypto("Invalid public key length".to_string())
            })?;
            let public_key = PublicKey::parse_compressed(&public_key_array).map_err(|e| {
                crate::error::KsError::Crypto(format!("Failed to parse compressed public key: {e}"))
            })?;
            info!(
                "Successfully generated key pair with key_id {} and expires_at: {}",
                response.key_id, response.expires_at
            );
            Ok((response.key_id, public_key, response.expires_at))
        } else {
            Err(crate::error::KsError::Crypto(format!(
                "Unsupported public key length: {}",
                public_key_bytes.len()
            )))
        }
    }

    /// 从 KS 服务获取私钥及过期时间
    pub async fn fetch_secret_key(
        &self,
        key_id: u32,
    ) -> Result<(SecretKey, u64), crate::error::KsError> {
        let url = format!("{}/secret/{}", self.endpoint, key_id);

        // 构建请求数据用于签名
        let request_data = format!("get_secret_key:{key_id}");

        // 创建 nonce credential
        let credential = CredentialBuilder::new(self.actrix_shared_key.as_bytes())
            .sign(request_data.as_bytes())?;

        // 构建查询参数
        let query_params = [
            ("key_id", key_id.to_string()),
            ("credential", serde_json::to_string(&credential)?),
        ];

        debug!("Fetching secret key {} from KS at {}", key_id, url);

        // 发送请求
        let response = self.client.get(&url).query(&query_params).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::error::KsError::Internal(format!(
                "KS request failed with status {status}: {error_text}"
            )));
        }

        // 解析响应
        let response: GetSecretKeyResponse = response.json().await?;

        // 解码私钥
        let secret_key_bytes = BASE64_STANDARD.decode(&response.secret_key)?;

        // SecretKey::parse 需要 &[u8; 32] 类型
        let secret_key_array: [u8; 32] = secret_key_bytes.try_into().map_err(|_| {
            crate::error::KsError::Crypto(
                "Invalid secret key length, expected 32 bytes".to_string(),
            )
        })?;

        let secret_key = SecretKey::parse(&secret_key_array).map_err(|e| {
            crate::error::KsError::Crypto(format!("Failed to parse secret key: {e}"))
        })?;

        info!(
            "Successfully fetched secret key {} from KS with expires_at: {}",
            key_id, response.expires_at
        );
        Ok((secret_key, response.expires_at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let config = ClientConfig {
            endpoint: "http://127.0.0.1:8090".to_string(),
            psk: "test-shared-key".to_string(),
            timeout_seconds: 30,
        };

        let client = Client::new(&config);
        assert_eq!(client.endpoint, "http://127.0.0.1:8090");
    }
}
