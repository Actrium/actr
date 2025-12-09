//! Realm 兼容性方法
//!
//! 提供与原有三个 Realm 表兼容的API，用于平滑迁移

use ecies::SecretKey;

use super::error::RealmError;
use super::model::Realm;

/// 兼容性方法 - 用于替换原有的三个表的功能
impl Realm {
    /// 替换原 RealmForAuthority::get_all_keys()
    pub async fn get_all_authority_keys() -> Result<Vec<Realm>, RealmError> {
        Self::get_all().await
    }

    /// 替换原 RealmForAuthority::get_keys()
    pub async fn get_authority_keys(
        key_id: String,
        realm_id: u32,
    ) -> Result<(Vec<u8>, Vec<u8>), RealmError> {
        let realm = Self::get_by_realm_key_id_service(realm_id, &key_id).await?;

        if let Some(t) = realm {
            let public_key = t.public_key;
            let secret_key = t.secret_key;
            Ok((public_key, secret_key))
        } else {
            Err(RealmError::NotFound)
        }
    }

    /// 替换原 RealmForSignaling::get_by_realm_id_and_key_id()
    pub async fn get_signaling_by_realm_id_and_key_id(
        realm_id: u32,
        key_id: &str,
    ) -> Result<SecretKey, RealmError> {
        let realm = Self::get_by_realm_key_id_service(realm_id, key_id)
            .await?
            .ok_or(RealmError::NotFound)?;
        SecretKey::parse_slice(realm.secret_key.as_slice())
            .map_err(|e| RealmError::ParseError(e.to_string()))
    }

    /// 替换原 RealmForTurn::get_private_key()
    pub async fn get_private_key(realm_id: u32, key_id: String) -> Result<SecretKey, RealmError> {
        let realm_opt = Self::get_by_realm_key_id_service(realm_id, &key_id).await?;

        if let Some(t) = realm_opt {
            if t.is_expired() {
                return Err(RealmError::KeyExpired);
            }

            SecretKey::parse_slice(t.secret_key.as_slice())
                .map_err(|e| RealmError::ParseError(e.to_string()))
        } else {
            Err(RealmError::KeyNotExist)
        }
    }

    /// 替换原 RealmForTurn::get_all_realms()
    pub async fn get_all_turn_realms() -> Result<Vec<Realm>, RealmError> {
        Self::get_all().await
    }
}
