use crate::MfrError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Status of a historical key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyHistoryStatus {
    /// Normal retirement: key rotated as part of regular lifecycle. Still valid for verifying old packages.
    Retired,
    /// Emergency revocation: private key compromised. Reject ALL verification attempts with this key.
    Revoked,
}

impl std::fmt::Display for KeyHistoryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retired => write!(f, "retired"),
            Self::Revoked => write!(f, "revoked"),
        }
    }
}

impl KeyHistoryStatus {
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "revoked" => Self::Revoked,
            _ => Self::Retired,
        }
    }
}

/// A retired MFR signing key, preserved for JWKS-style historical verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfrKeyHistory {
    pub id: i64,
    pub mfr_id: i64,
    pub key_id: String,
    pub public_key: String,
    pub status: KeyHistoryStatus,
    pub created_at: i64,
    pub retired_at: i64,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for MfrKeyHistory {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        let status_str: String = row
            .try_get("status")
            .unwrap_or_else(|_| "retired".to_string());
        Ok(MfrKeyHistory {
            id: row.try_get("id")?,
            mfr_id: row.try_get("mfr_id")?,
            key_id: row.try_get("key_id")?,
            public_key: row.try_get("public_key")?,
            status: KeyHistoryStatus::from_str_lossy(&status_str),
            created_at: row.try_get("created_at")?,
            retired_at: row.try_get("retired_at")?,
        })
    }
}

impl MfrKeyHistory {
    /// Archive a retired key into the history table.
    pub async fn archive(
        pool: &SqlitePool,
        mfr_id: i64,
        key_id: &str,
        public_key: &str,
        created_at: i64,
    ) -> Result<(), MfrError> {
        let now = Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO mfr_key_history (mfr_id, key_id, public_key, status, created_at, retired_at)
             VALUES (?, ?, ?, 'retired', ?, ?)",
        )
        .bind(mfr_id)
        .bind(key_id)
        .bind(public_key)
        .bind(created_at)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Revoke a historical key (emergency: private key compromised).
    /// After revocation, verification requests using this key_id will be rejected.
    pub async fn revoke(pool: &SqlitePool, history_id: i64) -> Result<(), MfrError> {
        let result = sqlx::query("UPDATE mfr_key_history SET status = 'revoked' WHERE id = ?")
            .bind(history_id)
            .execute(pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(MfrError::NotFound);
        }
        Ok(())
    }

    /// Look up a specific historical key by mfr_id + key_id.
    /// Returns None if the key is not found OR if it has been revoked.
    pub async fn get_by_key_id(
        pool: &SqlitePool,
        mfr_id: i64,
        key_id: &str,
    ) -> Result<Option<Self>, MfrError> {
        let entry = sqlx::query_as::<_, MfrKeyHistory>(
            "SELECT * FROM mfr_key_history WHERE mfr_id = ? AND key_id = ?",
        )
        .bind(mfr_id)
        .bind(key_id)
        .fetch_optional(pool)
        .await?;
        // Reject revoked keys: treat as if they don't exist
        match entry {
            Some(ref e) if e.status == KeyHistoryStatus::Revoked => {
                tracing::warn!(key_id, mfr_id, "rejected lookup for revoked historical key");
                Err(MfrError::KeyRevoked(key_id.to_string()))
            }
            other => Ok(other),
        }
    }

    /// Look up a specific historical key by mfr_id + key_id, regardless of status.
    /// Used by admin endpoints that need to see revoked keys.
    pub async fn get_by_key_id_unfiltered(
        pool: &SqlitePool,
        mfr_id: i64,
        key_id: &str,
    ) -> Result<Option<Self>, MfrError> {
        Ok(sqlx::query_as::<_, MfrKeyHistory>(
            "SELECT * FROM mfr_key_history WHERE mfr_id = ? AND key_id = ?",
        )
        .bind(mfr_id)
        .bind(key_id)
        .fetch_optional(pool)
        .await?)
    }

    /// List all historical keys for an MFR (most recent first).
    pub async fn list_by_mfr(pool: &SqlitePool, mfr_id: i64) -> Result<Vec<Self>, MfrError> {
        Ok(sqlx::query_as::<_, MfrKeyHistory>(
            "SELECT * FROM mfr_key_history WHERE mfr_id = ? ORDER BY retired_at DESC",
        )
        .bind(mfr_id)
        .fetch_all(pool)
        .await?)
    }

    /// Delete all history for an MFR (called when MFR is deleted).
    pub async fn delete_by_mfr(pool: &SqlitePool, mfr_id: i64) -> Result<(), MfrError> {
        sqlx::query("DELETE FROM mfr_key_history WHERE mfr_id = ?")
            .bind(mfr_id)
            .execute(pool)
            .await?;
        Ok(())
    }
}
