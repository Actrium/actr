use crate::MfrError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MfrStatus {
    #[default]
    Pending,
    Active,
    Suspended,
    Revoked,
}

impl std::fmt::Display for MfrStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MfrStatus::Pending => write!(f, "pending"),
            MfrStatus::Active => write!(f, "active"),
            MfrStatus::Suspended => write!(f, "suspended"),
            MfrStatus::Revoked => write!(f, "revoked"),
        }
    }
}

impl std::str::FromStr for MfrStatus {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "pending" => Ok(MfrStatus::Pending),
            "active" => Ok(MfrStatus::Active),
            "suspended" => Ok(MfrStatus::Suspended),
            "revoked" => Ok(MfrStatus::Revoked),
            _ => Err(()),
        }
    }
}

/// Manufacturer record.
/// `name` is the GitHub user/org login (lowercased) — it IS the identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manufacturer {
    pub id: i64,
    /// GitHub user/org login (lowercased), e.g. "octocat"
    pub name: String,
    pub public_key: String,
    pub contact: Option<String>,
    pub status: MfrStatus,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub verified_at: Option<i64>,
    pub suspended_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for Manufacturer {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        let status_str: String = row.try_get("status")?;
        let status = status_str.parse().unwrap_or_default();
        Ok(Manufacturer {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            public_key: row.try_get("public_key")?,
            contact: row.try_get("contact")?,
            status,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            verified_at: row.try_get("verified_at")?,
            suspended_at: row.try_get("suspended_at")?,
            revoked_at: row.try_get("revoked_at")?,
        })
    }
}

impl Manufacturer {
    pub async fn create(
        pool: &SqlitePool,
        name: &str,
        contact: Option<&str>,
    ) -> Result<Self, MfrError> {
        let now = Utc::now().timestamp();
        let id = sqlx::query(
            "INSERT INTO mfr (name, contact, status, created_at) VALUES (?, ?, 'pending', ?)",
        )
        .bind(name)
        .bind(contact)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                MfrError::AlreadyExists(format!("'{}' already registered", name))
            } else {
                MfrError::Database(e)
            }
        })?
        .last_insert_rowid();

        Self::get(pool, id).await?.ok_or(MfrError::NotFound)
    }

    pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Self>, MfrError> {
        Ok(
            sqlx::query_as::<_, Manufacturer>("SELECT * FROM mfr WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?,
        )
    }

    pub async fn get_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Self>, MfrError> {
        Ok(
            sqlx::query_as::<_, Manufacturer>("SELECT * FROM mfr WHERE name = ?")
                .bind(name)
                .fetch_optional(pool)
                .await?,
        )
    }

    pub async fn list(pool: &SqlitePool, status: Option<MfrStatus>) -> Result<Vec<Self>, MfrError> {
        if let Some(s) = status {
            Ok(sqlx::query_as::<_, Manufacturer>(
                "SELECT * FROM mfr WHERE status = ? ORDER BY created_at DESC",
            )
            .bind(s.to_string())
            .fetch_all(pool)
            .await?)
        } else {
            Ok(
                sqlx::query_as::<_, Manufacturer>("SELECT * FROM mfr ORDER BY created_at DESC")
                    .fetch_all(pool)
                    .await?,
            )
        }
    }

    pub async fn activate(
        &mut self,
        pool: &SqlitePool,
        public_key: String,
    ) -> Result<(), MfrError> {
        if self.status != MfrStatus::Pending {
            return Err(MfrError::InvalidStatus(format!(
                "cannot activate from status: {}",
                self.status
            )));
        }
        let now = Utc::now().timestamp();
        sqlx::query(
            "UPDATE mfr SET status = 'active', public_key = ?, verified_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&public_key)
        .bind(now)
        .bind(now)
        .bind(self.id)
        .execute(pool)
        .await?;
        self.status = MfrStatus::Active;
        self.public_key = public_key;
        self.verified_at = Some(now);
        self.updated_at = Some(now);
        Ok(())
    }

    pub async fn suspend(&mut self, pool: &SqlitePool) -> Result<(), MfrError> {
        if self.status != MfrStatus::Active {
            return Err(MfrError::InvalidStatus(format!(
                "cannot suspend from status: {}",
                self.status
            )));
        }
        let now = Utc::now().timestamp();
        sqlx::query(
            "UPDATE mfr SET status = 'suspended', suspended_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(self.id)
        .execute(pool)
        .await?;
        self.status = MfrStatus::Suspended;
        self.suspended_at = Some(now);
        Ok(())
    }

    pub async fn reinstate(&mut self, pool: &SqlitePool) -> Result<(), MfrError> {
        if self.status != MfrStatus::Suspended {
            return Err(MfrError::InvalidStatus(format!(
                "cannot reinstate from status: {}",
                self.status
            )));
        }
        let now = Utc::now().timestamp();
        sqlx::query("UPDATE mfr SET status = 'active', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(self.id)
            .execute(pool)
            .await?;
        self.status = MfrStatus::Active;
        self.updated_at = Some(now);
        Ok(())
    }

    pub async fn revoke(&mut self, pool: &SqlitePool) -> Result<(), MfrError> {
        let now = Utc::now().timestamp();
        sqlx::query(
            "UPDATE mfr SET status = 'revoked', revoked_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(self.id)
        .execute(pool)
        .await?;
        self.status = MfrStatus::Revoked;
        self.revoked_at = Some(now);
        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), MfrError> {
        sqlx::query("DELETE FROM mfr_package WHERE mfr_id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM mfr_challenge WHERE mfr_id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM mfr WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }
}
