//! # Config 模块
//!
//! 提供租户配置和访问控制列表管理功能。
//!
//! ## 主要组件
//!
//! - `TenantConfig`: 租户配置管理，支持键值对配置
//! - `ActorAcl`: Actor 访问控制列表，管理不同类型 Actor 之间的访问权限

//! ## 设计特点
//!
//! - 使用 sqlx 进行数据库操作
//! - 支持租户级别的配置隔离
//! - 提供灵活的访问控制机制

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::storage::db::get_database;
use crate::tenant::TenantError;

/// 租户配置结构体
///
/// 用于存储租户级别的键值对配置信息
#[derive(Debug, Clone, Serialize, Deserialize, Default, FromRow)]
pub struct TenantConfig {
    pub(crate) rowid: Option<i64>,
    pub(crate) tenant_id: i64,
    pub(crate) key: String,
    pub(crate) value: String,
}

impl TenantConfig {
    /// 创建新的租户配置
    pub fn new(tenant_id: i64, key: String, value: String) -> Self {
        Self {
            rowid: None,
            tenant_id,
            key,
            value,
        }
    }

    /// 保存配置到数据库
    pub async fn save(&mut self) -> Result<i64, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        if self.rowid.is_none() {
            // 插入新记录
            let result =
                sqlx::query("INSERT INTO tenantconfig (tenant_id, key, value) VALUES (?, ?, ?)")
                    .bind(self.tenant_id)
                    .bind(&self.key)
                    .bind(&self.value)
                    .execute(pool)
                    .await?;

            let new_rowid = result.last_insert_rowid();
            self.rowid = Some(new_rowid);
            Ok(new_rowid)
        } else {
            // 更新现有记录
            sqlx::query(
                "UPDATE tenantconfig SET tenant_id = ?, key = ?, value = ? WHERE rowid = ?",
            )
            .bind(self.tenant_id)
            .bind(&self.key)
            .bind(&self.value)
            .bind(self.rowid)
            .execute(pool)
            .await?;

            Ok(self.rowid.unwrap())
        }
    }

    /// 根据 ID 删除配置 (仅测试)
    #[cfg(test)]
    pub(crate) async fn delete_by_id(id: i64) -> Result<u64, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query("DELETE FROM tenantconfig WHERE rowid = ?")
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

    /// 根据 ID 获取配置 (仅测试)
    #[cfg(test)]
    pub(crate) async fn get(id: i64) -> Result<Option<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, TenantConfig>(
            "SELECT rowid, tenant_id, key, value FROM tenantconfig WHERE rowid = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    /// 获取指定租户的所有配置
    pub async fn get_by_tenant(tenant_id: i64) -> Result<Vec<Self>, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let configs = sqlx::query_as::<_, TenantConfig>(
            "SELECT rowid, tenant_id, key, value FROM tenantconfig WHERE tenant_id = ?",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;

        Ok(configs)
    }

    /// 根据租户 ID 和配置键获取配置
    pub async fn get_by_tenant_and_key(
        tenant_id: &str,
        key: &str,
    ) -> Result<Option<Self>, TenantError> {
        let tenant_id_i64: i64 = tenant_id
            .parse()
            .map_err(|e: std::num::ParseIntError| TenantError::ParseError(e.to_string()))?;

        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, TenantConfig>(
            "SELECT rowid, tenant_id, key, value FROM tenantconfig WHERE tenant_id = ? AND key = ?",
        )
        .bind(tenant_id_i64)
        .bind(key)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// 更新配置值
    pub fn set_value(&mut self, value: String) {
        self.value = value;
    }

    /// 删除指定租户的所有配置
    pub async fn delete_by_tenant(tenant_id: i64) -> Result<u64, TenantError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query("DELETE FROM tenantconfig WHERE tenant_id = ?")
            .bind(tenant_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tenant::Tenant, util::test_utils::utils::setup_test_db};
    use serial_test::serial;
    use uuid::Uuid;

    #[tokio::test]
    #[serial]
    async fn test_tenant_config_crud() -> anyhow::Result<()> {
        setup_test_db().await?;

        // Create a tenant first with unique name
        let tenant_id = format!("test_tenant_for_config_{}", Uuid::new_v4());
        let mut tenant = Tenant::new(
            tenant_id,
            "auth_key_for_config".to_string(),
            b"public_key".to_vec(),
            b"secret_key".to_vec(),
            "test_name".to_string(),
        );
        let tenant_row_id = tenant.save().await?;

        // Test create TenantConfig
        let mut config = TenantConfig::new(
            tenant_row_id,
            "test_key".to_string(),
            "test_value".to_string(),
        );
        let config_id = config.save().await?;
        assert!(config.rowid.is_some());
        assert_eq!(config.rowid.unwrap(), config_id);

        // Test get TenantConfig by ID
        let fetched_opt = TenantConfig::get(config_id).await?;
        assert!(fetched_opt.is_some());
        let fetched = fetched_opt.unwrap();
        assert_eq!(fetched.tenant_id, tenant_row_id);
        assert_eq!(fetched.key, "test_key");
        assert_eq!(fetched.value, "test_value");

        // Test update TenantConfig
        config.value = "updated_value".to_string();
        let updated_config_id = config.save().await?;
        assert_eq!(updated_config_id, config_id); // rowid should be the same

        let reloaded_opt = TenantConfig::get(config_id).await?;
        assert!(reloaded_opt.is_some());
        let reloaded = reloaded_opt.unwrap();
        assert_eq!(reloaded.value, "updated_value");

        // Test get_by_tenant
        let configs_for_tenant = TenantConfig::get_by_tenant(tenant_row_id).await?;
        assert_eq!(configs_for_tenant.len(), 1);
        assert_eq!(configs_for_tenant[0].rowid, Some(config_id));

        // Test get_by_tenant_and_key
        let config_by_key_opt =
            TenantConfig::get_by_tenant_and_key(&tenant_row_id.to_string(), "test_key").await?;
        assert!(config_by_key_opt.is_some());
        let config_by_key = config_by_key_opt.unwrap();
        assert_eq!(config_by_key.value, "updated_value");

        // Test delete TenantConfig
        TenantConfig::delete_by_id(config_id).await?;
        let deleted_config_opt = TenantConfig::get(config_id).await?;
        assert!(deleted_config_opt.is_none());

        Ok(())
    }
}
