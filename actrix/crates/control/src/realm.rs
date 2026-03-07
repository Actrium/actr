use actrix_proto::{RealmInfo, admin::v1::SecretRotationState};
use platform::realm::Realm;
use platform::storage::is_database_initialized;

use crate::error::AdminError;

/// Convert a Realm record into proto RealmInfo
pub fn realm_to_proto(realm: &Realm) -> RealmInfo {
    let secret_rotation_state = if !realm.secret_current.is_empty() {
        let (previous_hash, previous_valid_until) = match &realm.secret_previous {
            Some((hash, valid_until)) => (Some(hash.clone()), Some(*valid_until as i64)),
            None => (None, None),
        };
        Some(SecretRotationState {
            current_hash_preview: realm.secret_current.clone(),
            previous_hash_preview: previous_hash,
            previous_valid_until,
        })
    } else {
        None
    };

    RealmInfo {
        realm_id: realm.id,
        name: realm.name.clone(),
        enabled: realm.enabled,
        created_at: realm.created_at as i64,
        updated_at: realm.updated_at.map(|v| v as i64),
        expires_at: realm.expires_at.unwrap_or(0),
        status: realm.status.to_string(),
        secret_rotation_state,
    }
}

/// Get the maximum realm updated_at across all realms.
///
/// This is used to report the sync status to the Admin.
/// Returns 0 if the database is not initialized or no realms exist.
pub async fn get_max_realm_updated_at() -> Result<u64, AdminError> {
    if !is_database_initialized() {
        platform::recording::debug!(
            "Database not initialized, returning 0 for max realm updated_at"
        );
        return Ok(0);
    }

    let realms = match Realm::get_all().await {
        Ok(t) => t,
        Err(e) => {
            platform::recording::debug!("Failed to load realm list: {}", e);
            return Ok(0);
        }
    };

    let max_updated_at = realms
        .iter()
        .filter_map(|r| r.updated_at)
        .max()
        .unwrap_or(0);

    Ok(max_updated_at)
}
