//! Realm 验证逻辑
//!
//! 包含 Realm 相关的业务规则验证和检查

use super::model::{Realm, RealmStatus};

/// Realm 验证相关实现
impl Realm {
    /// 验证 Realm 是否可用（存在、未过期、状态正常）
    ///
    /// 返回 Ok(Realm) 表示 Realm 可用
    /// 返回 Err(msg) 表示 Realm 不可用，附带原因
    pub async fn validate_realm(realm_id: u32) -> Result<Realm, String> {
        let realm = Self::get(realm_id)
            .await
            .map_err(|e| format!("Failed to query realm: {e}"))?
            .ok_or_else(|| format!("Realm {realm_id} not found"))?;

        if realm.is_expired() {
            return Err(format!("Realm {realm_id} has expired"));
        }

        if realm.status != RealmStatus::Active {
            return Err(format!(
                "Realm {} is not in Active status (current: {})",
                realm_id, realm.status
            ));
        }

        if !realm.enabled {
            return Err(format!("Realm {realm_id} is disabled"));
        }

        Ok(realm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_expiration_check() {
        let realm = Realm {
            expires_at: Some(Utc::now().timestamp() as u64 - 3600),
            ..Default::default()
        };
        assert!(realm.is_expired());

        let realm2 = Realm::default();
        assert!(!realm2.is_expired());
    }

    #[test]
    fn test_is_active() {
        let realm = Realm {
            expires_at: Some(Utc::now().timestamp() as u64 + 3600),
            ..Default::default()
        };
        assert!(realm.is_active());

        let realm2 = Realm {
            expires_at: Some(Utc::now().timestamp() as u64 - 3600),
            ..Default::default()
        };
        assert!(!realm2.is_active());

        let realm3 = Realm {
            status: RealmStatus::Suspended,
            ..Default::default()
        };
        assert!(!realm3.is_active());
    }
}
