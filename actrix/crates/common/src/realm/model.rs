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
    pub key_id: String,
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
        key_id: String,
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
        let realm = Realm::new(
            12345,
            "test_key_id".to_string(),
            b"test_public".to_vec(),
            b"test_secret".to_vec(),
            "test_name".to_string(),
        );

        assert_eq!(realm.realm_id, 12345u32);
        assert_eq!(realm.key_id, "test_key_id");
        assert_eq!(realm.secret_key, b"test_secret".to_vec());
        assert_eq!(realm.public_key, b"test_public".to_vec());
        assert_eq!(realm.name, "test_name");
        assert!(realm.created_at.is_some());
        assert!(realm.updated_at.is_some());
    }

    #[test]
    fn test_authority_realm_creation() {
        let realm = Realm::new(
            54321,
            "auth_key_id".to_string(),
            b"auth_public".to_vec(),
            b"auth_secret".to_vec(),
            "Auth App".to_string(),
        );

        assert_eq!(realm.name, "Auth App");
        assert_eq!(realm.public_key, b"auth_public".to_vec());
    }
}
