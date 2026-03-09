//! 密钥存储后端抽象接口
//!
//! 定义了所有存储后端必须实现的统一异步接口

use crate::error::KsResult;
use crate::types::{KeyPair, KeyRecord};
use async_trait::async_trait;

/// 密钥存储后端抽象接口
///
/// 所有存储后端（SQLite, PostgreSQL）都需要实现此 trait
/// 提供统一的异步 API 用于密钥的生成、存储、查询和管理
/// 私钥永不离开存储后端，所有签名操作在后端内部完成
#[async_trait]
pub trait KeyStorageBackend: Send + Sync {
    /// 初始化存储后端
    ///
    /// 执行必要的初始化操作，如创建表、索引等
    async fn init(&self) -> KsResult<()>;

    /// 生成并存储新的 Ed25519 签名密钥对
    ///
    /// 自动生成 Ed25519 密钥对，存储到后端，返回包含 key_id 和 verifying key 的结构
    /// 私钥（signing key）只存储于后端，不返回给调用方
    ///
    /// # Returns
    /// 包含 key_id 和 verifying_key（Base64 编码的 32 字节验证公钥）的密钥对结构
    async fn generate_and_store_key(&self) -> KsResult<KeyPair>;

    /// 根据 key_id 查询验证公钥
    ///
    /// # Arguments
    /// * `key_id` - 密钥 ID
    ///
    /// # Returns
    /// * `Ok(Some(verifying_key))` - 找到验证公钥（Base64 编码）
    /// * `Ok(None)` - 密钥不存在
    /// * `Err(...)` - 存储错误
    async fn get_public_key(&self, key_id: u32) -> KsResult<Option<String>>;

    /// 使用指定密钥对消息进行 Ed25519 签名
    ///
    /// 私钥从存储中读取并解密，签名在内部完成，原始私钥不返回给调用方
    ///
    /// # Arguments
    /// * `key_id` - 密钥 ID
    /// * `message` - 待签名的消息字节
    ///
    /// # Returns
    /// * `Ok(signature)` - 64 字节 Ed25519 签名
    /// * `Err(KsError::NotFound)` - 密钥不存在
    /// * `Err(...)` - 存储或加密错误
    async fn sign(&self, key_id: u32, message: &[u8]) -> KsResult<Vec<u8>>;

    /// 获取完整的密钥记录（包含元数据）
    ///
    /// # Arguments
    /// * `key_id` - 密钥 ID
    ///
    /// # Returns
    /// * `Ok(Some(record))` - 找到密钥记录
    /// * `Ok(None)` - 密钥不存在
    /// * `Err(...)` - 存储错误
    async fn get_key_record(&self, key_id: u32) -> KsResult<Option<KeyRecord>>;

    /// 获取存储中的密钥总数
    ///
    /// # Returns
    /// 密钥总数（包括过期和未过期的）
    async fn get_key_count(&self) -> KsResult<u32>;

    /// 清理过期的密钥
    ///
    /// 删除所有已过期且超出容忍期的密钥记录
    /// （expires_at > 0 且 expires_at + tolerance_seconds < 当前时间）
    ///
    /// # Arguments
    /// * `tolerance_seconds` - 过期后的容忍期（秒）
    ///
    /// # Returns
    /// 被清理的密钥数量
    async fn cleanup_expired_keys(&self, tolerance_seconds: u64) -> KsResult<u32>;
}
