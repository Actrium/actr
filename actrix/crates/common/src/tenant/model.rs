//! Realm 核心数据结构
//!
//! 定义 Realm 实体的核心数据结构和基础方法

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Realm 是用于分离不同应用程序资源的虚拟概念。
///
#[derive(Debug, Clone, Serialize, Deserialize, Default, FromRow)]
pub struct Realm {
    pub rowid: Option<u32>,

    // 基础字段 - 所有服务都需要
    pub realm_id: u32,
    pub key_id: u32,
    pub secret_key: Vec<u8>,

    // 可选字段 - 部分服务需要
    pub name: String,        // Authority 服务需要
    pub public_key: Vec<u8>, // Authority 服务需要
    pub expires_at: Option<i64>,

    // 元数据字段
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

impl Realm {
    pub fn new(
        realm_id: u32,
        key_id: u32,
        public_key: Vec<u8>,
        secret_key: Vec<u8>,
        name: String,
    ) -> Self {
        let now = Utc::now().timestamp();
        Self {
            rowid: None,
            realm_id,
            key_id,
            secret_key,
            name,
            public_key,
            expires_at: None,
            created_at: Some(now),
            updated_at: Some(now),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn public_key(&self) -> &Vec<u8> {
        &self.public_key
    }

    pub fn secret_key(&self) -> &Vec<u8> {
        &self.secret_key
    }

    // Setter methods for admin operations
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn set_public_key(&mut self, public_key: Vec<u8>) {
        self.public_key = public_key;
    }

    pub fn set_secret_key(&mut self, secret_key: Vec<u8>) {
        self.secret_key = secret_key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_realm_creation() {
        let tenant = Realm::new(
            12345,
            1,
            b"test_public".to_vec(),
            b"test_secret".to_vec(),
            "test_name".to_string(),
        );

        assert_eq!(tenant.realm_id, 12345u32);
        assert_eq!(tenant.key_id, 1);
        assert_eq!(tenant.secret_key, b"test_secret".to_vec());
        assert_eq!(tenant.public_key, b"test_public".to_vec());
        assert_eq!(tenant.name, "test_name");
        assert!(tenant.created_at.is_some());
        assert!(tenant.updated_at.is_some());
    }

    #[test]
    fn test_authority_realm_creation() {
        let tenant = Realm::new(
            54321,
            2,
            b"auth_public".to_vec(),
            b"auth_secret".to_vec(),
            "Auth App".to_string(),
        );

        assert_eq!(tenant.name, "Auth App");
        assert_eq!(tenant.public_key, b"auth_public".to_vec());
    }
}
