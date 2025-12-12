//! KS 服务数据类型定义

use nonce_auth::NonceCredential;
use serde::{Deserialize, Serialize};

/// 密钥对结构
#[derive(Debug, Clone)]
pub struct KeyPair {
    /// 密钥 ID
    pub key_id: u32,
    /// 私钥（Base64 编码）
    pub secret_key: String,
    /// 公钥（Base64 编码）
    pub public_key: String,
}

/// 生成密钥请求
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateKeyRequest {
    /// nonce-auth 凭证
    pub credential: NonceCredential,
}

/// 生成密钥响应
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateKeyResponse {
    /// 生成的密钥 ID
    pub key_id: u32,
    /// 公钥（Base64 编码）
    pub public_key: String,
    /// 过期时间（Unix 时间戳）
    pub expires_at: u64,
    /// 容忍时间（秒）
    pub tolerance_seconds: u64,
}

/// 获取私钥请求
#[derive(Debug, Serialize, Deserialize)]
pub struct GetSecretKeyRequest {
    /// 要查询的密钥 ID
    pub key_id: u32,
    /// nonce-auth 凭证
    pub credential: NonceCredential,
}

/// 获取私钥响应
#[derive(Debug, Serialize, Deserialize)]
pub struct GetSecretKeyResponse {
    /// 密钥 ID
    pub key_id: u32,
    /// 私钥（Base64 编码）
    pub secret_key: String,
    /// 过期时间（Unix 时间戳）
    pub expires_at: u64,
    /// 是否在容忍期内（true = 密钥已过期但在容忍期，false = 正常有效期）
    pub in_tolerance_period: bool,
}

/// 存储在数据库中的密钥记录
#[derive(Debug, Clone, PartialEq)]
pub struct KeyRecord {
    /// 密钥 ID
    pub key_id: u32,
    /// 公钥（Base64 编码）
    pub public_key: String,
    /// 创建时间戳
    pub created_at: u64,
    /// 过期时间（Unix 时间戳）
    pub expires_at: u64,
}

impl GenerateKeyRequest {
    /// 获取用于验证的请求数据
    pub fn request_payload(&self) -> String {
        // 为生成密钥请求，我们只需要一个固定的标识符
        "generate_key".to_string()
    }
}

impl GetSecretKeyRequest {
    /// 获取用于验证的请求数据
    pub fn request_payload(&self) -> String {
        // 为获取私钥请求，我们包含密钥 ID
        format!("get_secret_key:{}", self.key_id)
    }
}
