//! 租户数据库操作
//!
//! 包含所有与租户数据持久化相关的CRUD操作

use chrono::Utc;

use super::error::TenantError;
use super::model::Tenant;
use crate::storage::db::get_database;

/// 租户数据库操作实现
impl Tenant {
    /// 根据租户ID、密钥ID获取租户密钥
    pub async fn get_keys(
        key_id: String,
        app_id: String,
    ) -> Result<(Vec<u8>, Vec<u8>), TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
            "SELECT public_key, secret_key FROM tenant WHERE tenant_id = ? AND key_id = ?",
        )
        .bind(&app_id)
        .bind(&key_id)
        .fetch_optional(pool)
        .await?;

        result.ok_or(TenantError::NotFound)
    }

    /// 保存租户到数据库
    /// 如果是新租户则插入，如果已存在提示已存在
    pub async fn save(&mut self) -> Result<i64, TenantError> {
        let now = Utc::now().timestamp();
        let db = get_database();
        let pool = db.get_pool();

        if self.rowid.is_none() {
            // 检查是否已存在相同的 tenant_id（应该全局唯一）
            let exists: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tenant WHERE tenant_id = ?")
                .bind(&self.tenant_id)
                .fetch_one(pool)
                .await?;

            if exists.0 > 0 {
                return Err(TenantError::DatabaseError(
                    "UNIQUE constraint failed: tenant.tenant_id".to_string(),
                ));
            }

            self.created_at = Some(now);
            self.updated_at = Some(now);

            // 插入新记录
            let result = sqlx::query(
                "INSERT INTO tenant (tenant_id, key_id, secret_key, name, public_key, expires_at, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&self.tenant_id)
            .bind(&self.key_id)
            .bind(&self.secret_key)
            .bind(&self.name)
            .bind(&self.public_key)
            .bind(self.expires_at)
            .bind(self.created_at)
            .bind(self.updated_at)
            .execute(pool)
            .await?;

            let new_rowid = result.last_insert_rowid();
            self.rowid = Some(new_rowid);
            Ok(new_rowid)
        } else {
            self.updated_at = Some(now);

            // 更新现有记录
            sqlx::query(
                "UPDATE tenant SET tenant_id = ?, key_id = ?, secret_key = ?, name = ?, public_key = ?, expires_at = ?, updated_at = ?
                 WHERE rowid = ?"
            )
            .bind(&self.tenant_id)
            .bind(&self.key_id)
            .bind(&self.secret_key)
            .bind(&self.name)
            .bind(&self.public_key)
            .bind(self.expires_at)
            .bind(self.updated_at)
            .bind(self.rowid)
            .execute(pool)
            .await?;

            self.rowid.ok_or_else(|| {
                TenantError::DatabaseError("Tenant rowid is missing after update".to_string())
            })
        }
    }

    /// 根据租户ID、密钥ID和服务类型获取租户
    pub async fn get_by_tenant_key_id_service(
        tenant_id: &str,
        key_id: &str,
    ) -> Result<Option<Tenant>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, Tenant>(
            "SELECT rowid, tenant_id, key_id, secret_key, name, public_key, expires_at, created_at, updated_at
             FROM tenant WHERE tenant_id = ? AND key_id = ?"
        )
        .bind(tenant_id)
        .bind(key_id)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// 获取所有租户
    pub async fn get_all() -> Result<Vec<Tenant>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let tenants = sqlx::query_as::<_, Tenant>(
            "SELECT rowid, tenant_id, key_id, secret_key, name, public_key, expires_at, created_at, updated_at
             FROM tenant"
        )
        .fetch_all(pool)
        .await?;

        tracing::info!("获取所有租户: {:?}", tenants);
        Ok(tenants)
    }

    /// 根据删除租户
    pub async fn delete_instance(tenant_id: String) -> Result<u64, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query("DELETE FROM tenant WHERE tenant_id = ?")
            .bind(tenant_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// 根据 ID 获取租户
    pub async fn get(id: i64) -> Result<Option<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, Tenant>(
            "SELECT rowid, tenant_id, key_id, secret_key, name, public_key, expires_at, created_at, updated_at
             FROM tenant WHERE rowid = ?"
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// 根据名称获取租户
    pub async fn get_by_name(name: &str) -> Result<Option<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, Tenant>(
            "SELECT rowid, tenant_id, key_id, secret_key, name, public_key, expires_at, created_at, updated_at
             FROM tenant WHERE name = ?"
        )
        .bind(name)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// 根据租户ID获取租户
    pub async fn get_by_tenant_id(tenant_id: &str) -> Result<Option<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, Tenant>(
            "SELECT rowid, tenant_id, key_id, secret_key, name, public_key, expires_at, created_at, updated_at
             FROM tenant WHERE tenant_id = ?",
        )
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// 获取所有租户列表
    pub async fn list() -> Result<Vec<Self>, TenantError> {
        Self::get_all().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_utils::utils::setup_test_db;
    use serial_test::serial;
    use uuid::Uuid;

    #[tokio::test]
    #[serial]
    async fn test_database_schema() -> anyhow::Result<()> {
        setup_test_db().await?;

        // 创建一个租户来触发表创建，使用唯一名称
        let tenant_id = format!("test_schema_{}", Uuid::new_v4());
        let mut tenant = Tenant::new(
            tenant_id,
            "auth_key".to_string(),
            b"public_key".to_vec(),
            b"secret_key".to_vec(),
            "test_name".to_string(),
        );
        let _tenant_id = tenant.save().await?;

        // 查询表结构
        let db = get_database();
        let pool = db.get_pool();

        let schema_info: Option<(String,)> =
            sqlx::query_as("SELECT sql FROM sqlite_master WHERE type='table' AND name='tenant'")
                .fetch_optional(pool)
                .await?;
        println!("Schema query result: {schema_info:?}");

        // 查询索引信息
        let index_info: Vec<(String,)> = sqlx::query_as(
            "SELECT sql FROM sqlite_master WHERE type='index' AND tbl_name='tenant'",
        )
        .fetch_all(pool)
        .await?;
        println!("Index query result: {index_info:?}");

        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn test_duplicate_tenant_name() -> anyhow::Result<()> {
        setup_test_db().await?;

        let tenant_name = format!("duplicate_test_{}", Uuid::new_v4());

        let mut tenant1 = Tenant::new(
            tenant_name.clone(),
            "key_id1".to_string(),
            b"public_key".to_vec(),
            b"secret_key".to_vec(),
            "test_name".to_string(),
        );
        let tenant1_id = tenant1.save().await?; // Save first tenant
        println!("Created first tenant with ID: {tenant1_id}");

        // Try to create another tenant with the same name
        let mut tenant2 = Tenant::new(
            tenant_name.clone(),
            "auth2".to_string(),
            b"public_key".to_vec(),
            b"secret_key".to_vec(),
            "test_name".to_string(),
        );
        let result = tenant2.save().await;

        println!("Second tenant save result: {result:?}");

        // Should fail due to duplicate name
        assert!(result.is_err());
        if let Err(TenantError::DatabaseError(msg)) = result {
            println!("Got database error: {msg}");
            assert!(msg.contains("UNIQUE constraint failed") || msg.contains("already exists"));
        } else {
            panic!("Expected DatabaseError for duplicate tenant name, got: {result:?}");
        }

        Ok(())
    }
}
