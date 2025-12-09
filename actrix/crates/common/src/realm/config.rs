//! # Config 模块
//!
//! 提供 Realm 配置和访问控制列表管理功能。
//!
//! ## 主要组件
//!
//! - `RealmConfig`: Realm 配置管理，支持键值对配置
//! - `ActorAcl`: Actor 访问控制列表，管理不同类型 Actor 之间的访问权限

//! ## 设计特点
//!
//! - 使用 sqlx 进行数据库操作
//! - 支持 Realm 级别的配置隔离
//! - 提供灵活的访问控制机制

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::realm::RealmError;
use crate::storage::db::get_database;

/// 用于存储 Realm 级别的键值对配置信息
#[derive(Debug, Clone, Serialize, Deserialize, Default, FromRow)]
pub struct RealmConfig {
    pub(crate) rowid: Option<u32>,
    pub(crate) realm_id: u32, // 存储 realm.rowid
    pub(crate) key: String,
    pub(crate) value: String,
}

impl RealmConfig {
    pub fn new(realm_id: u32, key: String, value: String) -> Self {
        Self {
            rowid: None,
            realm_id,
            key,
            value,
        }
    }

    pub async fn save(&mut self) -> Result<u32, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        if self.rowid.is_none() {
            // 插入新记录
            let result =
                sqlx::query("INSERT INTO realmconfig (realm_id, key, value) VALUES (?, ?, ?)")
                    .bind(self.realm_id as i64)
                    .bind(&self.key)
                    .bind(&self.value)
                    .execute(pool)
                    .await?;

            let new_rowid = result.last_insert_rowid().try_into().unwrap();
            self.rowid = Some(new_rowid);
            Ok(new_rowid)
        } else {
            // 更新现有记录
            sqlx::query("UPDATE realmconfig SET realm_id = ?, key = ?, value = ? WHERE rowid = ?")
                .bind(self.realm_id as i64)
                .bind(&self.key)
                .bind(&self.value)
                .bind(self.rowid)
                .execute(pool)
                .await?;

            Ok(self.rowid.unwrap())
        }
    }

    #[cfg(test)]
    pub(crate) async fn delete_by_id(id: u32) -> Result<u64, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query("DELETE FROM realmconfig WHERE rowid = ?")
            .bind(id as i64)
            .execute(pool)
            .await?;

        let changes = result.rows_affected();
        if changes > 0 {
            Ok(changes)
        } else {
            Err(RealmError::NotFound)
        }
    }

    #[cfg(test)]
    pub(crate) async fn get(id: u32) -> Result<Option<Self>, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let result = sqlx::query_as::<_, RealmConfig>(
            "SELECT rowid, realm_id, key, value FROM realmconfig WHERE rowid = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(result)
    }

    pub async fn get_by_realm(realm_id: u32) -> Result<Vec<Self>, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let realm_id_i64 = realm_id as i64;
        let rows =
            sqlx::query("SELECT rowid, realm_id, key, value FROM realmconfig WHERE realm_id = ?")
                .bind(realm_id_i64)
                .fetch_all(pool)
                .await?;

        let mut configs = Vec::new();
        for row in rows {
            configs.push(RealmConfig::from_row(&row)?);
        }
        Ok(configs)
    }

    /// 注意：这里的 realm_id 是 realm.rowid
    pub async fn get_by_realm_and_key(
        realm_id: u32,
        key: &str,
    ) -> Result<Option<Self>, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let realm_id_i64 = realm_id as i64;
        let result = sqlx::query(
            "SELECT rowid, realm_id, key, value FROM realmconfig WHERE realm_id = ? AND key = ?",
        )
        .bind(realm_id_i64)
        .bind(key)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = result {
            Ok(Some(RealmConfig::from_row(&row)?))
        } else {
            Ok(None)
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: String) {
        self.value = value;
    }

    pub async fn delete_by_realm(realm_id: u32) -> Result<u64, RealmError> {
        let db = get_database();
        let pool = db.get_pool();

        let realm_id_i64 = realm_id as i64;
        let result = sqlx::query("DELETE FROM realmconfig WHERE realm_id = ?")
            .bind(realm_id_i64)
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{realm::Realm, util::test_utils::utils::setup_test_db};
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn test_realm_config_crud() -> anyhow::Result<()> {
        setup_test_db().await?;

        // Create a realm first with unique name
        let realm_id = rand::random::<u32>();
        let mut realm = Realm::new(
            realm_id,
            "auth_key_for_config".to_string(),
            b"public_key".to_vec(),
            b"secret_key".to_vec(),
            "test_name".to_string(),
        );
        let realm_row_id = realm.save().await?;

        // Test create RealmConfig
        let mut config = RealmConfig::new(
            realm_row_id,
            "test_key".to_string(),
            "test_value".to_string(),
        );
        let config_id = config.save().await?;
        assert!(config.rowid.is_some());
        assert_eq!(config.rowid.unwrap(), config_id);

        // Test get RealmConfig by ID
        let fetched_opt = RealmConfig::get(config_id).await?;
        assert!(fetched_opt.is_some());
        let fetched = fetched_opt.unwrap();
        assert_eq!(fetched.realm_id, realm_row_id);
        assert_eq!(fetched.key, "test_key");
        assert_eq!(fetched.value, "test_value");

        // Test update RealmConfig
        config.value = "updated_value".to_string();
        let updated_config_id = config.save().await?;
        assert_eq!(updated_config_id, config_id); // rowid should be the same

        let reloaded_opt = RealmConfig::get(config_id).await?;
        assert!(reloaded_opt.is_some());
        let reloaded = reloaded_opt.unwrap();
        assert_eq!(reloaded.value, "updated_value");

        // Test get_by_realm
        let configs_for_realm = RealmConfig::get_by_realm(realm_row_id).await?;
        assert_eq!(configs_for_realm.len(), 1);
        assert_eq!(configs_for_realm[0].rowid, Some(config_id));

        // Test get_by_realm_and_key
        let config_by_key_opt = RealmConfig::get_by_realm_and_key(realm_row_id, "test_key").await?;
        assert!(config_by_key_opt.is_some());
        let config_by_key = config_by_key_opt.unwrap();
        assert_eq!(config_by_key.value, "updated_value");

        // Test delete RealmConfig
        RealmConfig::delete_by_id(config_id).await?;
        let deleted_config_opt = RealmConfig::get(config_id).await?;
        assert!(deleted_config_opt.is_none());

        Ok(())
    }
}
