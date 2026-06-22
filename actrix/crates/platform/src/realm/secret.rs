//! Realm secret 管理
//!
//! 用于 Admin 分配、AIS 校验 realm secret。
//! Secret 数据直接存储在 Realm 表的 secret_current / secret_previous 字段中。

use super::{Realm, RealmError};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// signaling -> AIS 传递 realm secret 的 HTTP 头
pub const REALM_SECRET_HEADER: &str = "x-actrix-realm-secret";

/// 旧 secret 的默认兼容窗口（4 小时）
pub const DEFAULT_REALM_SECRET_PREVIOUS_GRACE_SECS: u64 = 4 * 3600;

/// Realm secret 校验结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmSecretCheck {
    /// realm 未配置 secret（兼容历史 realm）
    NotConfigured,
    /// 命中当前 secret
    ValidCurrent,
    /// 命中上一代 secret（且仍在兼容窗口内）
    ValidPrevious,
    /// realm 已配置 secret，但请求未提供
    MissingRequired,
    /// 请求提供了 secret，但不匹配当前/可用上一代
    Invalid,
}

/// 轮转结果（仅返回明文给调用方一次）
#[derive(Debug, Clone)]
pub struct RealmSecretRotation {
    pub new_secret: String,
    pub previous_valid_until: Option<u64>,
}

/// Secret 状态元数据
#[derive(Debug, Clone, Default)]
pub struct RealmSecretState {
    pub current_hash: Option<String>,
    pub previous_hash: Option<String>,
    pub previous_valid_until: Option<u64>,
}

/// 生成用于加入 realm 的 secret（URL-safe，便于 CLI / URL 透传）
pub fn generate_realm_secret() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("rs_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Hash a realm secret with SHA256.
pub fn hash_realm_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// 轮转 realm secret（旧 secret 进入兼容窗口）
///
/// 加载 Realm → current→previous → 生成新 hash → save
pub async fn rotate_realm_secret(
    realm_id: u32,
    previous_grace_secs: Option<u64>,
) -> Result<RealmSecretRotation, RealmError> {
    let grace_secs = previous_grace_secs.unwrap_or(DEFAULT_REALM_SECRET_PREVIOUS_GRACE_SECS);

    let mut realm = Realm::get(realm_id).await?.ok_or(RealmError::NotFound)?;

    let now = Utc::now().timestamp() as u64;
    let previous_valid_until = now.saturating_add(grace_secs);

    // Move current to previous (if current is non-empty)
    if !realm.secret_current.is_empty() {
        realm.secret_previous = Some((realm.secret_current.clone(), previous_valid_until));
    } else {
        realm.secret_previous = None;
    }

    // Generate new secret
    let new_secret = generate_realm_secret();
    let new_hash = hash_realm_secret(&new_secret);
    realm.secret_current = new_hash;

    realm.save().await?;

    Ok(RealmSecretRotation {
        new_secret,
        previous_valid_until: Some(previous_valid_until),
    })
}

/// 获取 realm secret 状态元数据（仅 hash，不含明文）
pub async fn get_realm_secret_state(realm_id: u32) -> Result<RealmSecretState, RealmError> {
    let realm = Realm::get(realm_id).await?.ok_or(RealmError::NotFound)?;

    let current_hash = if realm.secret_current.is_empty() {
        None
    } else {
        Some(realm.secret_current.clone())
    };

    let (previous_hash, previous_valid_until) = match &realm.secret_previous {
        Some((hash, valid_until)) => (Some(hash.clone()), Some(*valid_until)),
        None => (None, None),
    };

    Ok(RealmSecretState {
        current_hash,
        previous_hash,
        previous_valid_until,
    })
}

/// 按 realm id 校验请求 secret。
pub async fn verify_realm_secret(
    realm_id: u32,
    provided_secret: Option<&str>,
) -> Result<RealmSecretCheck, RealmError> {
    let realm = Realm::get(realm_id).await?.ok_or(RealmError::NotFound)?;

    // 历史 realm 未配置 secret：保持兼容
    if realm.secret_current.is_empty() {
        return Ok(RealmSecretCheck::NotConfigured);
    }

    let provided = provided_secret.map(str::trim).filter(|v| !v.is_empty());
    let Some(provided) = provided else {
        return Ok(RealmSecretCheck::MissingRequired);
    };

    let provided_hash = hash_realm_secret(provided);
    if provided_hash == realm.secret_current {
        return Ok(RealmSecretCheck::ValidCurrent);
    }

    // Check previous secret within grace window
    if let Some((prev_hash, valid_until)) = &realm.secret_previous {
        let now = Utc::now().timestamp() as u64;
        if now <= *valid_until && provided_hash == *prev_hash {
            return Ok(RealmSecretCheck::ValidPrevious);
        }
    }

    Ok(RealmSecretCheck::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_utils::utils::setup_test_db;
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn test_create_and_verify_secret() -> anyhow::Result<()> {
        setup_test_db().await?;

        let secret = generate_realm_secret();
        let hash = hash_realm_secret(&secret);
        let realm = Realm::create("test-secret-realm".to_string(), hash).await?;

        let result = verify_realm_secret(realm.id, Some(&secret)).await?;
        assert_eq!(result, RealmSecretCheck::ValidCurrent);

        let missing = verify_realm_secret(realm.id, None).await?;
        assert_eq!(missing, RealmSecretCheck::MissingRequired);

        let invalid = verify_realm_secret(realm.id, Some("wrong-secret")).await?;
        assert_eq!(invalid, RealmSecretCheck::Invalid);

        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn test_rotate_secret_keeps_previous_temporarily_valid() -> anyhow::Result<()> {
        setup_test_db().await?;

        let old_secret = generate_realm_secret();
        let old_hash = hash_realm_secret(&old_secret);
        let realm = Realm::create("test-rotate-realm".to_string(), old_hash).await?;

        let rotated = rotate_realm_secret(realm.id, Some(60)).await?;

        let new_ok = verify_realm_secret(realm.id, Some(&rotated.new_secret)).await?;
        assert_eq!(new_ok, RealmSecretCheck::ValidCurrent);

        let old_ok = verify_realm_secret(realm.id, Some(&old_secret)).await?;
        assert_eq!(old_ok, RealmSecretCheck::ValidPrevious);

        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn test_not_configured_secret() -> anyhow::Result<()> {
        setup_test_db().await?;

        // Create realm with empty secret
        let realm = Realm::create("test-no-secret".to_string(), String::new()).await?;

        let result = verify_realm_secret(realm.id, None).await?;
        assert_eq!(result, RealmSecretCheck::NotConfigured);

        Ok(())
    }
}
