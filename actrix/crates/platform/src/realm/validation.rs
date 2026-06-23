//! Realm 验证逻辑
//!
//! 包含 Realm 相关的业务规则验证和检查

use super::model::{Realm, RealmStatus};

/// Structured realm validation error.
///
/// `RealmUnavailable` maps to HTTP 403: the realm exists but is not usable.
/// `StoreError` maps to HTTP 500: the database query itself failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmValidationError {
    /// Realm not found, disabled, expired, or in a non-Active status.
    RealmUnavailable { realm_id: u32, reason: String },
    /// Database error during realm lookup.
    StoreError { realm_id: u32, message: String },
}

impl std::fmt::Display for RealmValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RealmUnavailable { realm_id, reason } => {
                write!(f, "Realm {realm_id} unavailable: {reason}")
            }
            Self::StoreError { realm_id, message } => {
                write!(f, "Database error for realm {realm_id}: {message}")
            }
        }
    }
}

impl std::error::Error for RealmValidationError {}

/// Realm 验证相关实现
impl Realm {
    /// 验证 Realm 是否可用（存在、未过期、状态正常）
    ///
    /// 返回 Ok(Realm) 表示 Realm 可用
    /// 返回 Err(RealmValidationError) 表示 Realm 不可用或数据库错误
    pub async fn validate_realm(realm_id: u32) -> Result<Realm, RealmValidationError> {
        let realm = Self::get(realm_id)
            .await
            .map_err(|e| RealmValidationError::StoreError {
                realm_id,
                message: format!("Failed to query realm: {e}"),
            })?;

        let Some(realm) = realm else {
            return Err(RealmValidationError::RealmUnavailable {
                realm_id,
                reason: format!("Realm {realm_id} not found"),
            });
        };

        if realm.is_expired() {
            return Err(RealmValidationError::RealmUnavailable {
                realm_id,
                reason: format!("Realm {realm_id} has expired"),
            });
        }

        if realm.status != RealmStatus::Active {
            return Err(RealmValidationError::RealmUnavailable {
                realm_id,
                reason: format!(
                    "Realm {} is not in Active status (current: {})",
                    realm_id, realm.status
                ),
            });
        }

        if !realm.enabled {
            return Err(RealmValidationError::RealmUnavailable {
                realm_id,
                reason: format!("Realm {realm_id} is disabled"),
            });
        }

        Ok(realm)
    }

    /// Map a [RealmValidationError] to the appropriate HTTP status code
    /// and a human-readable error string (for backward-compatible callers).
    ///
    /// - `RealmUnavailable` → (403, reason)
    /// - `StoreError` → (500, message)
    pub fn map_validation_error(err: RealmValidationError) -> (u32, String) {
        match &err {
            RealmValidationError::RealmUnavailable { reason, .. } => (403, reason.clone()),
            RealmValidationError::StoreError { message, .. } => (500, message.clone()),
        }
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

    #[test]
    fn test_validation_error_display() {
        let err = RealmValidationError::RealmUnavailable {
            realm_id: 1,
            reason: "test reason".to_string(),
        };
        assert_eq!(err.to_string(), "Realm 1 unavailable: test reason");
        assert_eq!(
            Realm::map_validation_error(err),
            (403, "test reason".to_string())
        );

        let err2 = RealmValidationError::StoreError {
            realm_id: 2,
            message: "timeout".to_string(),
        };
        assert_eq!(err2.to_string(), "Database error for realm 2: timeout");
        assert_eq!(
            Realm::map_validation_error(err2),
            (500, "timeout".to_string())
        );
    }
}
