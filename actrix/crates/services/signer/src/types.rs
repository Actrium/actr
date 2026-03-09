//! KS 服务数据类型定义

use nonce_auth::NonceCredential;
use serde::{Deserialize, Serialize};

/// 密钥对结构（内部使用，存储公钥引用）
#[derive(Debug, Clone)]
pub struct KeyPair {
    /// 密钥 ID
    pub key_id: u32,
    /// 验证公钥（Base64 编码的 Ed25519 verifying key，32 字节）
    pub verifying_key: String,
}

/// 生成签名密钥请求
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateSigningKeyRequest {
    /// nonce-auth 凭证
    pub credential: NonceCredential,
}

/// 生成签名密钥响应
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateSigningKeyResponse {
    /// 生成的密钥 ID
    pub key_id: u32,
    /// 验证公钥（Base64 编码的 Ed25519 verifying key，32 字节）
    pub verifying_key: String,
    /// 过期时间（Unix 时间戳）
    pub expires_at: u64,
    /// 容忍时间（秒）
    pub tolerance_seconds: u64,
}

/// 签名请求
#[derive(Debug, Serialize, Deserialize)]
pub struct SignRequest {
    /// 要使用的密钥 ID
    pub key_id: u32,
    /// 待签名的消息（原始字节，base64 编码用于 HTTP 传输）
    pub message: Vec<u8>,
    /// nonce-auth 凭证
    pub credential: NonceCredential,
}

/// 签名响应
#[derive(Debug, Serialize, Deserialize)]
pub struct SignResponse {
    /// Ed25519 签名（64 字节）
    pub signature: Vec<u8>,
}

/// 存储在数据库中的密钥记录
#[derive(Debug, Clone, PartialEq)]
pub struct KeyRecord {
    /// 密钥 ID
    pub key_id: u32,
    /// 验证公钥（Base64 编码的 Ed25519 verifying key）
    pub public_key: String,
    /// 创建时间戳
    pub created_at: u64,
    /// 过期时间（Unix 时间戳）
    pub expires_at: u64,
}

impl GenerateSigningKeyRequest {
    /// 获取用于验证的请求数据
    pub fn request_payload(&self) -> String {
        "generate_signing_key".to_string()
    }
}

impl SignRequest {
    /// 获取用于验证的请求数据
    pub fn request_payload(&self) -> String {
        format!("sign:{}", self.key_id)
    }
}
