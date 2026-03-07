//! SQLite-backed config override store
//!
//! Persists dynamic configuration overrides (L2) in the `config_overrides` table
//! within `actrix.db`. Validates against the config registry before writing.

use super::registry;
use anyhow::{Result, bail};
use serde::Serialize;
use sqlx::sqlite::SqlitePool;

/// A single config override record.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ConfigOverride {
    pub key_path: String,
    pub value: String,
    pub updated_at: String,
    pub updated_by: String,
}

/// SQLite-backed store for dynamic config overrides.
#[derive(Clone, Debug)]
pub struct ConfigOverrideStore {
    pool: SqlitePool,
}

impl ConfigOverrideStore {
    /// Create a new store, creating the `config_overrides` table if needed.
    pub async fn new(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS config_overrides (
                key_path    TEXT PRIMARY KEY,
                value       TEXT NOT NULL,
                updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_by  TEXT NOT NULL DEFAULT 'admin'
            )",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    /// Get a single override by key.
    pub async fn get(&self, key: &str) -> Result<Option<ConfigOverride>> {
        let row = sqlx::query_as::<_, ConfigOverride>(
            "SELECT key_path, value, updated_at, updated_by FROM config_overrides WHERE key_path = ?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Set (upsert) an override value. Validates against the registry.
    pub async fn set(&self, key: &str, value: &str, by: &str) -> Result<()> {
        let field =
            registry::get_field(key).ok_or_else(|| anyhow::anyhow!("Unknown config key: {key}"))?;

        if !field.dynamic {
            bail!(
                "Config key '{}' is not dynamic and cannot be overridden at runtime",
                key
            );
        }

        if !field.validate(value) {
            bail!(
                "Invalid value '{}' for config key '{}' (expected type: {}{})",
                value,
                key,
                field.value_type,
                if field.choices.is_empty() {
                    String::new()
                } else {
                    format!(", choices: [{}]", field.choices.join(", "))
                }
            );
        }

        sqlx::query(
            "INSERT INTO config_overrides (key_path, value, updated_at, updated_by)
             VALUES (?, ?, datetime('now'), ?)
             ON CONFLICT(key_path) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at,
                updated_by = excluded.updated_by",
        )
        .bind(key)
        .bind(value)
        .bind(by)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete an override. Returns true if a row was actually deleted.
    pub async fn delete(&self, key: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM config_overrides WHERE key_path = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// List all overrides.
    pub async fn list_all(&self) -> Result<Vec<ConfigOverride>> {
        let rows = sqlx::query_as::<_, ConfigOverride>(
            "SELECT key_path, value, updated_at, updated_by FROM config_overrides ORDER BY key_path",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn test_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_crud() {
        let pool = test_pool().await;
        let store = ConfigOverrideStore::new(pool).await.unwrap();

        // Initially empty
        assert!(store.list_all().await.unwrap().is_empty());

        // Set a dynamic field
        store
            .set("turn.realm", "test.local", "admin")
            .await
            .unwrap();

        // Get it back
        let entry = store.get("turn.realm").await.unwrap().unwrap();
        assert_eq!(entry.value, "test.local");
        assert_eq!(entry.updated_by, "admin");

        // Update it
        store
            .set("turn.realm", "updated.local", "admin")
            .await
            .unwrap();
        let entry = store.get("turn.realm").await.unwrap().unwrap();
        assert_eq!(entry.value, "updated.local");

        // List
        let all = store.list_all().await.unwrap();
        assert_eq!(all.len(), 1);

        // Delete
        assert!(store.delete("turn.realm").await.unwrap());
        assert!(store.get("turn.realm").await.unwrap().is_none());
        assert!(!store.delete("turn.realm").await.unwrap());
    }

    #[tokio::test]
    async fn test_rejects_non_dynamic() {
        let pool = test_pool().await;
        let store = ConfigOverrideStore::new(pool).await.unwrap();

        let result = store.set("bind.ice.ip", "1.2.3.4", "admin").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not dynamic"));
    }

    #[tokio::test]
    async fn test_rejects_invalid_type() {
        let pool = test_pool().await;
        let store = ConfigOverrideStore::new(pool).await.unwrap();

        let result = store
            .set(
                "services.signaling.server.rate_limit.connection.per_minute",
                "not_a_number",
                "admin",
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid value"));
    }

    #[tokio::test]
    async fn test_rejects_unknown_key() {
        let pool = test_pool().await;
        let store = ConfigOverrideStore::new(pool).await.unwrap();

        let result = store.set("nonexistent.key", "value", "admin").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown config key")
        );
    }
}
