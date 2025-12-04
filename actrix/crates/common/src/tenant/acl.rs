//! Actor 访问控制列表
//!
//! 定义了 Actor 的权限控制数据结构
use anyhow::Result;

use serde::{Deserialize, Serialize};

use super::super::TenantError;
use crate::storage::db::get_database;

const ANONYMOUS_ACTOR_TYPE: &str = "ANONCLNT";
const VOICE_ACTOR_TYPE: &str = "VOICE";
const CHAT_ACTOR_TYPE: &str = "CHAT";

/// Actor 访问控制列表
///
/// 管理不同类型 Actor 之间的访问权限
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ActorAcl {
    pub rowid: Option<i64>,
    pub tenant_id: String,
    pub from_type: String,
    pub to_type: String,
    pub access: bool,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for ActorAcl {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            rowid: row.try_get("rowid")?,
            tenant_id: row.try_get("tenant_id")?,
            from_type: row.try_get("from_type")?,
            to_type: row.try_get("to_type")?,
            access: row.try_get::<i64, _>("access")? != 0,
        })
    }
}

impl ActorAcl {
    /// 创建新的访问控制规则
    pub fn new(tenant_id: String, from_type: String, to_type: String, access: bool) -> Self {
        Self {
            rowid: None,
            tenant_id,
            from_type: from_type.to_string(),
            to_type: to_type.to_string(),
            access,
        }
    }

    /// Save ACL rule to database
    ///
    /// Inserts a new rule or updates existing one based on rowid.
    ///
    /// # Returns
    ///
    /// Returns the rowid of the saved ACL rule
    pub async fn save(&mut self) -> Result<i64, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        if self.rowid.is_none() {
            // 插入新记录
            let result = sqlx::query(
                "INSERT INTO actoracl (tenant_id, from_type, to_type, access) VALUES (?, ?, ?, ?)",
            )
            .bind(&self.tenant_id)
            .bind(&self.from_type)
            .bind(&self.to_type)
            .bind(if self.access { 1 } else { 0 })
            .execute(pool)
            .await?;

            let new_rowid = result.last_insert_rowid();
            self.rowid = Some(new_rowid);
            Ok(new_rowid)
        } else {
            // 更新现有记录
            sqlx::query(
                "UPDATE actoracl SET tenant_id = ?, from_type = ?, to_type = ?, access = ? WHERE rowid = ?"
            )
            .bind(&self.tenant_id)
            .bind(&self.from_type)
            .bind(&self.to_type)
            .bind(if self.access { 1 } else { 0 })
            .bind(self.rowid)
            .execute(pool)
            .await?;

            Ok(self.rowid.unwrap())
        }
    }

    /// Delete ACL rule by ID
    ///
    /// # Arguments
    ///
    /// - `id`: ACL rule rowid
    ///
    /// # Returns
    ///
    /// Returns number of deleted rows (0 or 1)
    pub async fn delete_by_id(id: i64) -> Result<u64, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query("DELETE FROM actoracl WHERE rowid = ?")
            .bind(id)
            .execute(pool)
            .await?;

        let changes = result.rows_affected();
        if changes > 0 {
            Ok(changes)
        } else {
            Err(TenantError::NotFound)
        }
    }

    /// Get ACL rule by ID
    ///
    /// # Arguments
    ///
    /// - `id`: ACL rule rowid
    ///
    /// # Returns
    ///
    /// Returns the ACL rule if found, None otherwise
    pub async fn get(id: i64) -> Result<Option<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, ActorAcl>(
            "SELECT rowid, tenant_id, from_type, to_type, access FROM actoracl WHERE rowid = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// 获取指定租户的所有访问控制规则
    pub async fn get_by_tenant(tenant_id: String) -> Result<Vec<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let acls = sqlx::query_as::<_, ActorAcl>(
            "SELECT rowid, tenant_id, from_type, to_type, access FROM actoracl WHERE tenant_id = ?",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;

        Ok(acls)
    }

    /// 根据类型获取访问控制规则
    pub async fn get_by_types(
        tenant_id: &str,
        from_type: &str,
        to_type: &str,
    ) -> Result<Option<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, ActorAcl>(
            "SELECT rowid, tenant_id, from_type, to_type, access FROM actoracl
             WHERE tenant_id = ? AND from_type = ? AND to_type = ?",
        )
        .bind(tenant_id)
        .bind(from_type)
        .bind(to_type)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    pub fn access(&self) -> bool {
        self.access
    }

    /// Check if discovery is allowed between two actor types
    ///
    /// Used for Presence notification filtering and service discovery
    ///
    /// # Arguments
    ///
    /// - `tenant_id`: Tenant ID
    /// - `from_type`: Source actor type
    /// - `to_type`: Target actor type
    ///
    /// # Returns
    ///
    /// Returns true if discovery is allowed, false otherwise.
    /// Default policy: deny if no rule exists (secure by default)
    pub async fn can_discover(
        tenant_id: &str,
        from_type: &str,
        to_type: &str,
    ) -> Result<bool, TenantError> {
        match Self::get_by_types(tenant_id, from_type, to_type).await? {
            Some(acl) => {
                tracing::debug!(
                    tenant_id = %tenant_id,
                    from_type = %from_type,
                    to_type = %to_type,
                    access = acl.access(),
                    "ACL rule found"
                );
                Ok(acl.access())
            }
            None => {
                // Default policy: deny if no rule exists
                tracing::debug!(
                    tenant_id = %tenant_id,
                    from_type = %from_type,
                    to_type = %to_type,
                    "No ACL rule found, denying discovery (default policy)"
                );
                Ok(false)
            }
        }
    }
}

pub fn mock_actor_acl() -> Vec<ActorAcl> {
    vec![
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: ANONYMOUS_ACTOR_TYPE.to_string(),
            to_type: VOICE_ACTOR_TYPE.to_string(),
            access: true,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: ANONYMOUS_ACTOR_TYPE.to_string(),
            to_type: CHAT_ACTOR_TYPE.to_string(),
            access: true,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: ANONYMOUS_ACTOR_TYPE.to_string(),
            to_type: ANONYMOUS_ACTOR_TYPE.to_string(),
            access: false,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: CHAT_ACTOR_TYPE.to_string(),
            to_type: ANONYMOUS_ACTOR_TYPE.to_string(),
            access: true,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: CHAT_ACTOR_TYPE.to_string(),
            to_type: VOICE_ACTOR_TYPE.to_string(),
            access: true,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: CHAT_ACTOR_TYPE.to_string(),
            to_type: CHAT_ACTOR_TYPE.to_string(),
            access: true,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: VOICE_ACTOR_TYPE.to_string(),
            to_type: ANONYMOUS_ACTOR_TYPE.to_string(),
            access: true,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: VOICE_ACTOR_TYPE.to_string(),
            to_type: CHAT_ACTOR_TYPE.to_string(),
            access: true,
        },
        ActorAcl {
            rowid: None,
            tenant_id: "2".to_string(),
            from_type: VOICE_ACTOR_TYPE.to_string(),
            to_type: VOICE_ACTOR_TYPE.to_string(),
            access: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::Tenant;
    use crate::util::test_utils::utils::setup_test_db;
    use serial_test::serial;
    use uuid::Uuid;

    #[tokio::test]
    #[serial]
    async fn test_actor_acl_crud() -> anyhow::Result<()> {
        setup_test_db().await?;

        // Create a tenant first with unique name
        let tenant_id = format!("test_tenant_for_acl_{}", Uuid::new_v4());
        let mut tenant = Tenant::new(
            tenant_id,
            "auth_key_for_acl".to_string(),
            b"public_key".to_vec(),
            b"secret_key".to_vec(),
            "test_name".to_string(),
        );
        let tenant_row_id = tenant.save().await?;

        // Test create
        let mut acl = ActorAcl::new(
            tenant_row_id.to_string(),
            "identified_client_user".to_string(),
            "identified_client_room".to_string(),
            true,
        );
        let acl_id = acl.save().await?;
        assert!(acl.rowid.is_some());
        assert_eq!(acl.rowid.unwrap(), acl_id);

        // Test get
        let fetched_opt = ActorAcl::get(acl_id).await?;
        assert!(fetched_opt.is_some());
        let fetched = fetched_opt.unwrap();
        assert_eq!(fetched.tenant_id, tenant_row_id.to_string());
        assert_eq!(fetched.from_type, "identified_client_user");
        assert_eq!(fetched.to_type, "identified_client_room");
        assert!(fetched.access);

        // Test update
        acl.access = false;
        let updated_acl_id = acl.save().await?;
        assert_eq!(updated_acl_id, acl_id);

        let reloaded_opt = ActorAcl::get(acl_id).await?;
        assert!(reloaded_opt.is_some());
        let reloaded = reloaded_opt.unwrap();
        assert!(!reloaded.access);

        // Test get_by_tenant
        let acls_for_tenant = ActorAcl::get_by_tenant(tenant_row_id.to_string()).await?;
        assert_eq!(acls_for_tenant.len(), 1);
        assert_eq!(acls_for_tenant[0].rowid, Some(acl_id));

        // Test get_by_types
        let acl_by_types_opt = ActorAcl::get_by_types(
            &tenant_row_id.to_string(),
            "identified_client_user",
            "identified_client_room",
        )
        .await?;
        assert!(acl_by_types_opt.is_some());
        let acl_by_types = acl_by_types_opt.unwrap();
        assert!(!acl_by_types.access);

        // Test delete
        ActorAcl::delete_by_id(acl_id).await?;
        let deleted_acl_opt = ActorAcl::get(acl_id).await?;
        assert!(deleted_acl_opt.is_none());

        Ok(())
    }
}
