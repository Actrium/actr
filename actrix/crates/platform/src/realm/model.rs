//! Realm 核心数据结构与数据库操作
//!
//! 定义 Realm 实体的核心数据结构、数据库 CRUD 操作

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use strum::{Display, EnumString};

use super::error::RealmError;
use crate::storage::db::get_database;

/// Realm 状态枚举
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Display, EnumString)]
pub enum RealmStatus {
    #[default]
    Active,
    Inactive,
    Suspended,
}

/// Realm 是用于分离不同应用程序资源的虚拟概念。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Realm {
    /// DB 自增主键，起点 2^25 = 33554432
    pub id: u32,
    pub name: String,
    pub status: RealmStatus,
    pub enabled: bool,
    pub expires_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: Option<u64>,
    /// SHA256 hash of current secret (必填)
    pub secret_current: String,
    /// (hash, valid_until) for previous secret during rotation grace window
    pub secret_previous: Option<(String, u64)>,
}

impl Default for Realm {
    fn default() -> Self {
        let now = Utc::now().timestamp() as u64;
        Self {
            id: 0,
            name: String::new(),
            status: RealmStatus::Active,
            enabled: true,
            expires_at: None,
            created_at: now,
            updated_at: None,
            secret_current: String::new(),
            secret_previous: None,
        }
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for Realm {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        let id: i64 = row.try_get("id")?;
        let name: String = row.try_get("name")?;
        let status_str: String = row.try_get("status")?;
        let enabled: bool = row.try_get::<i32, _>("enabled")? != 0;
        let expires_at: Option<i64> = row.try_get("expires_at")?;
        let created_at: i64 = row.try_get("created_at")?;
        let updated_at: Option<i64> = row.try_get("updated_at")?;
        let secret_current: String = row.try_get("secret_current")?;
        let secret_previous_hash: Option<String> = row.try_get("secret_previous_hash")?;
        let secret_previous_valid_until: Option<i64> =
            row.try_get("secret_previous_valid_until")?;

        let secret_previous = match (secret_previous_hash, secret_previous_valid_until) {
            (Some(hash), Some(valid_until)) if !hash.is_empty() => Some((hash, valid_until as u64)),
            _ => None,
        };

        Ok(Self {
            id: id as u32,
            name,
            status: RealmStatus::from_str(&status_str).unwrap_or_default(),
            enabled,
            expires_at: expires_at.map(|v| v as u64),
            created_at: created_at as u64,
            updated_at: updated_at.map(|v| v as u64),
            secret_current,
            secret_previous,
        })
    }
}

impl Realm {
    /// Create a new realm in the database.
    ///
    /// `secret_hash` is the SHA256 hash of the realm secret (must be non-empty).
    /// Returns the Realm with its auto-generated `id`.
    pub async fn create(name: String, secret_hash: String) -> Result<Self, RealmError> {
        let db = get_database();
        let pool = db.get_pool();
        let now = Utc::now().timestamp();

        let result = sqlx::query(
            "INSERT INTO realm (name, status, enabled, created_at, secret_current)
             VALUES (?, 'Active', 1, ?, ?)",
        )
        .bind(&name)
        .bind(now)
        .bind(&secret_hash)
        .execute(pool)
        .await?;

        let id = result.last_insert_rowid() as u32;

        Ok(Self {
            id,
            name,
            status: RealmStatus::Active,
            enabled: true,
            expires_at: None,
            created_at: now as u64,
            updated_at: None,
            secret_current: secret_hash,
            secret_previous: None,
        })
    }

    /// Save (UPDATE) an existing realm to the database.
    pub async fn save(&mut self) -> Result<(), RealmError> {
        let db = get_database();
        let pool = db.get_pool();
        let now = Utc::now().timestamp();
        self.updated_at = Some(now as u64);

        let (prev_hash, prev_valid_until) = match &self.secret_previous {
            Some((hash, valid_until)) => (Some(hash.as_str()), Some(*valid_until as i64)),
            None => (None, None),
        };

        sqlx::query(
            "UPDATE realm SET name = ?, status = ?, enabled = ?, expires_at = ?,
             updated_at = ?, secret_current = ?,
             secret_previous_hash = ?, secret_previous_valid_until = ?
             WHERE id = ?",
        )
        .bind(&self.name)
        .bind(self.status.to_string())
        .bind(self.enabled as i32)
        .bind(self.expires_at.map(|v| v as i64))
        .bind(now)
        .bind(&self.secret_current)
        .bind(prev_hash)
        .bind(prev_valid_until)
        .bind(self.id as i64)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Get a realm by its auto-increment id.
    pub async fn get(id: u32) -> Result<Option<Self>, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, Realm>(
            "SELECT id, name, status, enabled, expires_at, created_at, updated_at,
                    secret_current, secret_previous_hash, secret_previous_valid_until
             FROM realm WHERE id = ?",
        )
        .bind(id as i64)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// Get a realm by name.
    pub async fn get_by_name(name: &str) -> Result<Option<Self>, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, Realm>(
            "SELECT id, name, status, enabled, expires_at, created_at, updated_at,
                    secret_current, secret_previous_hash, secret_previous_valid_until
             FROM realm WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// Get all realms.
    pub async fn get_all() -> Result<Vec<Self>, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let realms = sqlx::query_as::<_, Realm>(
            "SELECT id, name, status, enabled, expires_at, created_at, updated_at,
                    secret_current, secret_previous_hash, secret_previous_valid_until
             FROM realm",
        )
        .fetch_all(pool)
        .await?;

        Ok(realms)
    }

    /// Delete a realm by id.
    pub async fn delete(id: u32) -> Result<u64, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query("DELETE FROM realm WHERE id = ?")
            .bind(id as i64)
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }

    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = Utc::now().timestamp() as u64;
            now > expires_at
        } else {
            false
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == RealmStatus::Active && self.enabled && !self.is_expired()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_realm_status_display() {
        assert_eq!(RealmStatus::Active.to_string(), "Active");
        assert_eq!(RealmStatus::Inactive.to_string(), "Inactive");
        assert_eq!(RealmStatus::Suspended.to_string(), "Suspended");
    }

    #[test]
    fn test_realm_status_parse() {
        assert_eq!(
            RealmStatus::from_str("Active").unwrap(),
            RealmStatus::Active
        );
        assert_eq!(
            RealmStatus::from_str("Inactive").unwrap(),
            RealmStatus::Inactive
        );
        assert_eq!(
            RealmStatus::from_str("Suspended").unwrap(),
            RealmStatus::Suspended
        );
    }

    #[test]
    fn test_realm_default() {
        let realm = Realm::default();
        assert_eq!(realm.id, 0);
        assert_eq!(realm.status, RealmStatus::Active);
        assert!(realm.enabled);
        assert!(realm.secret_current.is_empty());
        assert!(realm.secret_previous.is_none());
    }

    #[test]
    fn test_realm_expired() {
        let mut realm = Realm::default();
        let past_time = Utc::now().timestamp() as u64 - 3600;
        realm.expires_at = Some(past_time);
        assert!(realm.is_expired());
        assert!(!realm.is_active());
    }

    #[test]
    fn test_realm_active() {
        let mut realm = Realm::default();
        let future_time = Utc::now().timestamp() as u64 + 3600;
        realm.expires_at = Some(future_time);
        assert!(!realm.is_expired());
        assert!(realm.is_active());
    }

    #[test]
    fn test_realm_suspended_not_active() {
        let realm = Realm {
            status: RealmStatus::Suspended,
            ..Default::default()
        };
        assert!(!realm.is_active());
    }

    #[test]
    fn test_realm_disabled_not_active() {
        let realm = Realm {
            enabled: false,
            ..Default::default()
        };
        assert!(!realm.is_active());
    }
}
