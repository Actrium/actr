//! Realm 验证逻辑
//!
//! 包含 Realm 相关的业务规则验证和检查

use chrono::Utc;

use super::model::Realm;

/// Realm 验证相关实现
impl Realm {
    /// 检查 Realm 是否存在
    pub async fn exists(realm_id: u32, key_id: u32) -> bool {
        Self::get_by_realm_key_id_service(realm_id, key_id)
            .await
            .unwrap_or(None)
            .is_some()
    }

    /// 验证密钥
    pub fn verify_secret_key(&self, secret_key: &Vec<u8>) -> bool {
        self.secret_key == *secret_key
    }

    /// 检查是否过期（用于 Turn 服务）
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            expires_at < Utc::now().timestamp()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expiration_check() {
        let past_time = Utc::now().timestamp() - 3600; // 1 hour ago
        let mut tenant = Realm::new(
            99999,
            1,
            b"expired_public".to_vec(),
            b"expired_secret".to_vec(),
            "Expired App".to_string(),
        );

        // Set expired time to test expiration
        tenant.expires_at = Some(past_time);
        assert!(tenant.is_expired());

        // Test non-expiring realm
        tenant.expires_at = None;
        assert!(!tenant.is_expired());
    }

    #[test]
    fn test_verify_secret_key() {
        let tenant = Realm::new(
            12345,
            1,
            b"correct_public".to_vec(),
            b"correct_secret".to_vec(),
            "test_name".to_string(),
        );

        assert!(tenant.verify_secret_key(&b"correct_secret".to_vec()));
        assert!(!tenant.verify_secret_key(&b"wrong_secret".to_vec()));
    }
}
