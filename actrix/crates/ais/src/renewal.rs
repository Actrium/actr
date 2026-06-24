//! AIS renewal token management — deterministic rotation, storage, and cleanup.
//!
//! # Design
//!
//! Renewal tokens are bound to a specific `ActrId` and stored only as
//! SHA-256(token) in SQLite. Successor tokens are derived deterministically
//! via HMAC-SHA256 so concurrent requests and retries produce the same token.
//!
//! Rotation happens in a single SQLite transaction:
//! 1. Look up the unexpired token by `actor_id + token_hash`.
//! 2. If remaining lifetime > rotation window, return the current token unchanged.
//! 3. Otherwise derive the successor deterministically and `INSERT OR IGNORE`.
//! 4. Delete expired tokens for this actor; keep at most the 2 most recent tokens.
//!
//! # Security
//!
//! - The database never stores raw tokens.
//! - Logging must never emit tokens, token hashes, or realm secrets.

use actr_protocol::ActrId;
use hmac::{Hmac, Mac};
use prost::bytes::Bytes;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

// -------- public API ---------------------------------------------------------

/// Compute SHA-256(token) for storage lookup.
pub fn hash_token(token: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(token);
    hasher.finalize().to_vec()
}

/// Deterministically derive a successor renewal token.
///
/// ```text
/// HMAC-SHA256(
///   renewal_token_secret,
///   "actrix/ais/renewal/v1" ||
///   old_token ||
///   ActrId::to_string_repr() ||
///   old_expires_at_be
/// )
/// ```
///
/// `old_expires_at` is encoded as big-endian u64 bytes for portability.
pub fn derive_successor_token(
    secret: &[u8],
    old_token: &[u8],
    actor_id: &ActrId,
    old_expires_at: i64,
) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length");
    mac.update(b"actrix/ais/renewal/v1");
    mac.update(old_token);
    mac.update(actor_id.to_string_repr().as_bytes());
    mac.update(&old_expires_at.to_be_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Compute successor expiry: old_expires_at + ttl_secs.
///
/// Returned as Unix seconds (i64) for direct SQLite storage.
pub fn successor_expires_at(old_expires_at: i64, ttl_secs: u64) -> i64 {
    old_expires_at + ttl_secs as i64
}

/// Current Unix timestamp in seconds (truncated to i64 for SQLite).
pub fn now_secs() -> i64 {
    try_now_secs().unwrap_or_default() as i64
}

/// Current Unix timestamp in seconds, preserving a pre-epoch clock error for
/// callers that must reject invalid timestamps instead of falling back to zero.
pub fn try_now_secs() -> Result<u64, std::time::SystemTimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
}

// -------- database operations ------------------------------------------------

/// Persist a renewal token hash for the given actor.
///
/// Uses the shared `actrix.db` pool. Token hash uniqueness enforces that
/// duplicate inserts are silently ignored (`INSERT OR IGNORE`).
pub async fn insert_renewal_token(
    actor_id: &ActrId,
    token_hash: &[u8],
    expires_at: i64,
) -> Result<(), sqlx::Error> {
    let pool = platform::storage::db::get_database().get_pool();
    let now = now_secs();

    // Use a separate connection for this write — the caller handles
    // the enclosing transaction for rotation.
    sqlx::query(
        "INSERT OR IGNORE INTO ais_renewal_token (actor_id, token_hash, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(actor_id.to_string_repr())
    .bind(token_hash)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

/// Row returned by renewal token queries.
pub struct RenewalTokenRow {
    #[allow(dead_code)]
    pub id: i64,
    pub actor_id: String,
    pub token_hash: Vec<u8>,
    pub expires_at: i64,
}

/// Look up an unexpired token by `actor_id + token_hash`.
pub async fn find_unexpired_token(
    actor_id: &ActrId,
    token_hash: &[u8],
) -> Result<Option<RenewalTokenRow>, sqlx::Error> {
    let pool = platform::storage::db::get_database().get_pool();
    let now = now_secs();

    let row = sqlx::query_as::<_, (i64, String, Vec<u8>, i64)>(
        "SELECT id, actor_id, token_hash, expires_at
         FROM ais_renewal_token
         WHERE actor_id = ?1 AND token_hash = ?2 AND expires_at > ?3
         LIMIT 1",
    )
    .bind(actor_id.to_string_repr())
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?;

    Ok(
        row.map(|(id, actor_id, token_hash, expires_at)| RenewalTokenRow {
            id,
            actor_id,
            token_hash,
            expires_at,
        }),
    )
}

/// Delete all expired tokens for a given actor.
pub async fn delete_expired_tokens_for_actor(actor_id: &ActrId) -> Result<u64, sqlx::Error> {
    let pool = platform::storage::db::get_database().get_pool();
    let now = now_secs();

    let result =
        sqlx::query("DELETE FROM ais_renewal_token WHERE actor_id = ?1 AND expires_at <= ?2")
            .bind(actor_id.to_string_repr())
            .bind(now)
            .execute(pool)
            .await?;

    Ok(result.rows_affected())
}

/// Delete all but the 2 most recent tokens for a given actor.
pub async fn trim_oldest_tokens_for_actor(actor_id: &ActrId) -> Result<u64, sqlx::Error> {
    let pool = platform::storage::db::get_database().get_pool();

    // SQLite doesn't support LIMIT in subqueries for DELETE directly,
    // so we use a rowid-based approach.
    let result = sqlx::query(
        "DELETE FROM ais_renewal_token
         WHERE actor_id = ?1
           AND id NOT IN (
               SELECT id FROM ais_renewal_token
               WHERE actor_id = ?1
               ORDER BY expires_at DESC, created_at DESC
               LIMIT 2
           )",
    )
    .bind(actor_id.to_string_repr())
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Global GC: delete all expired tokens across all actors.
///
/// Called periodically (low-frequency) and at the start of each
/// register/renew transaction.
pub async fn gc_expired_tokens() -> Result<u64, sqlx::Error> {
    let pool = platform::storage::db::get_database().get_pool();
    let now = now_secs();

    let result = sqlx::query("DELETE FROM ais_renewal_token WHERE expires_at <= ?1")
        .bind(now)
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}

// -------- rotation logic -----------------------------------------------------

/// Outcome of a renewal token rotation attempt.
pub enum RotationOutcome {
    /// Token was still fresh — return the same token and expiry.
    Unchanged { token: Bytes, expires_at: i64 },
    /// Entered the rotation window — return the successor token and its expiry.
    Rotated { token: Bytes, expires_at: i64 },
}

/// Execute the full rotation transaction.
///
/// 1. Verify the old token exists and is unexpired (401 otherwise).
/// 2. If remaining lifetime > rotation_window_secs, return `Unchanged`.
/// 3. Otherwise derive successor, `INSERT OR IGNORE` successor hash.
/// 4. Delete expired tokens, trim to 2 newest.
/// 5. Return `Rotated` with the successor token and expiry.
pub async fn rotate_renewal_token(
    actor_id: &ActrId,
    old_token: &[u8],
    secret: &[u8],
    rotation_window_secs: u64,
    ttl_secs: u64,
) -> Result<RotationOutcome, RenewalError> {
    let old_hash = hash_token(old_token);
    let now = now_secs();
    let actor_id_repr = actor_id.to_string_repr();
    let pool = platform::storage::db::get_database().get_pool();
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| RenewalError::StoreError(format!("begin transaction failed: {e}")))?;

    // 1. Look up the unexpired token inside the transaction.
    let row = sqlx::query_as::<_, (i64, String, Vec<u8>, i64)>(
        "SELECT id, actor_id, token_hash, expires_at
         FROM ais_renewal_token
         WHERE actor_id = ?1 AND token_hash = ?2 AND expires_at > ?3
         LIMIT 1",
    )
    .bind(&actor_id_repr)
    .bind(&old_hash)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| RenewalError::StoreError(format!("token lookup failed: {e}")))?;

    let Some((_, _, _, current_expires_at)) = row else {
        tx.rollback()
            .await
            .map_err(|e| RenewalError::StoreError(format!("rollback failed: {e}")))?;
        return Err(RenewalError::TokenRejected);
    };

    // 2. If remaining lifetime > rotation window, return unchanged.
    let remaining = current_expires_at - now;
    if remaining > rotation_window_secs as i64 {
        tx.commit()
            .await
            .map_err(|e| RenewalError::StoreError(format!("commit failed: {e}")))?;
        return Ok(RotationOutcome::Unchanged {
            token: Bytes::copy_from_slice(old_token),
            expires_at: current_expires_at,
        });
    }

    // 3. Derive successor.
    let successor = derive_successor_token(secret, old_token, actor_id, current_expires_at);
    let successor_hash = hash_token(&successor);
    let successor_expires = successor_expires_at(current_expires_at, ttl_secs);

    // 4. Insert successor hash (INSERT OR IGNORE — idempotent).
    sqlx::query(
        "INSERT OR IGNORE INTO ais_renewal_token (actor_id, token_hash, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&actor_id_repr)
    .bind(&successor_hash)
    .bind(successor_expires)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| RenewalError::StoreError(format!("failed to insert successor: {e}")))?;

    // 5. Cleanup — expired tokens + keep at most 2 newest.
    sqlx::query("DELETE FROM ais_renewal_token WHERE actor_id = ?1 AND expires_at <= ?2")
        .bind(&actor_id_repr)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| RenewalError::StoreError(format!("failed to delete expired tokens: {e}")))?;

    sqlx::query(
        "DELETE FROM ais_renewal_token
         WHERE actor_id = ?1
           AND id NOT IN (
               SELECT id FROM ais_renewal_token
               WHERE actor_id = ?1
               ORDER BY expires_at DESC, created_at DESC
               LIMIT 2
           )",
    )
    .bind(&actor_id_repr)
    .execute(&mut *tx)
    .await
    .map_err(|e| RenewalError::StoreError(format!("failed to trim old tokens: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| RenewalError::StoreError(format!("commit failed: {e}")))?;

    platform::recording::info!(
        "Renewal token rotated for actor {}: old_expires={}, new_expires={}",
        actor_id_repr,
        current_expires_at,
        successor_expires
    );

    Ok(RotationOutcome::Rotated {
        token: Bytes::from(successor),
        expires_at: successor_expires,
    })
}

/// Errors specific to renewal token management.
#[derive(Debug, Clone)]
pub enum RenewalError {
    /// Token not found or expired — maps to HTTP 401.
    TokenRejected,
    /// Database error — maps to HTTP 500.
    StoreError(String),
}

impl std::fmt::Display for RenewalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TokenRejected => write!(f, "renewal token rejected"),
            Self::StoreError(msg) => write!(f, "renewal store error: {msg}"),
        }
    }
}

impl std::error::Error for RenewalError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_token_deterministic() {
        let token = b"test-token-32-bytes-xxxxxxxxxxx";
        let h1 = hash_token(token);
        let h2 = hash_token(token);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 32); // SHA-256
    }

    #[test]
    fn test_hash_token_different_inputs() {
        let h1 = hash_token(b"token-a");
        let h2 = hash_token(b"token-b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_successor_expiry_calculation() {
        let old = 1_000_000;
        let ttl = 86_400;
        assert_eq!(successor_expires_at(old, ttl), 1_086_400);
    }

    #[test]
    fn test_derive_successor_deterministic() {
        let secret = b"test-secret-at-least-32-bytes!";
        let old = b"old-renewal-token-32-bytes-xx";
        let actor_id = ActrId {
            realm: actr_protocol::Realm { realm_id: 1 },
            serial_number: 42,
            r#type: actr_protocol::ActrType {
                manufacturer: "test".to_string(),
                name: "actor".to_string(),
                version: "1.0.0".to_string(),
            },
        };

        let s1 = derive_successor_token(secret, old, &actor_id, 1_000_000);
        let s2 = derive_successor_token(secret, old, &actor_id, 1_000_000);
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 32);
    }

    #[test]
    fn test_derive_successor_different_old_expiry() {
        let secret = b"test-secret-at-least-32-bytes!";
        let old = b"old-renewal-token-32-bytes-xx";
        let actor_id = ActrId {
            realm: actr_protocol::Realm { realm_id: 1 },
            serial_number: 42,
            r#type: actr_protocol::ActrType {
                manufacturer: "test".to_string(),
                name: "actor".to_string(),
                version: "1.0.0".to_string(),
            },
        };

        let s1 = derive_successor_token(secret, old, &actor_id, 1_000_000);
        let s2 = derive_successor_token(secret, old, &actor_id, 1_000_001);
        assert_ne!(s1, s2);
    }
}
