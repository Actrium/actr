//! KS 客户端 - 简单的 HTTP 客户端（仅用于测试）

use crate::types::{GenerateSigningKeyRequest, GenerateSigningKeyResponse, SignRequest, SignResponse};
use base64::prelude::*;
use ed25519_dalek::VerifyingKey;
use nonce_auth::CredentialBuilder;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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

    /// 从 KS 服务生成新的 Ed25519 签名密钥对
    ///
    /// 返回 (key_id, verifying_key_bytes[32], expires_at)
    /// 私钥保留在 KS 服务端
    pub async fn generate_signing_key(
        &self,
    ) -> Result<(u32, VerifyingKey, u64), crate::error::KsError> {
        let url = format!("{}/generate-signing-key", self.endpoint);
        let request_data = "generate_signing_key";

        let credential = CredentialBuilder::new(self.actrix_shared_key.as_bytes())
            .sign(request_data.as_bytes())?;

        let request = GenerateSigningKeyRequest { credential };

        crate::recording::debug!("Requesting Ed25519 signing key generation from KS at {}", url);

        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::error::KsError::Internal(format!(
                "KS generate signing key request failed with status {status}: {error_text}"
            )));
        }

        let response: GenerateSigningKeyResponse = response.json().await?;

        let vk_bytes = BASE64_STANDARD.decode(&response.verifying_key)?;
        let vk_array: [u8; 32] = vk_bytes.try_into().map_err(|_| {
            crate::error::KsError::Crypto("Invalid verifying key length, expected 32 bytes".to_string())
        })?;

        let verifying_key = VerifyingKey::from_bytes(&vk_array)
            .map_err(|e| crate::error::KsError::Crypto(format!("Invalid Ed25519 verifying key: {e}")))?;

        crate::recording::info!(
            "Successfully generated Ed25519 signing key with key_id={}, expires_at={}",
            response.key_id,
            response.expires_at
        );
        Ok((response.key_id, verifying_key, response.expires_at))
    }

    /// 使用 KS 服务中的密钥对消息进行签名
    ///
    /// 返回 64 字节 Ed25519 签名
    pub async fn sign(
        &self,
        key_id: u32,
        message: &[u8],
    ) -> Result<Vec<u8>, crate::error::KsError> {
        let url = format!("{}/sign/{}", self.endpoint, key_id);
        let request_data = format!("sign:{key_id}");

        let credential = CredentialBuilder::new(self.actrix_shared_key.as_bytes())
            .sign(request_data.as_bytes())?;

        let request = SignRequest {
            key_id,
            message: message.to_vec(),
            credential,
        };

        crate::recording::debug!("Requesting signature for key_id={} from KS at {}", key_id, url);

        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::error::KsError::Internal(format!(
                "KS sign request failed with status {status}: {error_text}"
            )));
        }

        let response: SignResponse = response.json().await?;

        if response.signature.len() != 64 {
            return Err(crate::error::KsError::Crypto(format!(
                "Invalid signature length: expected 64 bytes, got {}",
                response.signature.len()
            )));
        }

        crate::recording::info!("Successfully obtained signature for key_id={} from KS", key_id);
        Ok(response.signature)
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
