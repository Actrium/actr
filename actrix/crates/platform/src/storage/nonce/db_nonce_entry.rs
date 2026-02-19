//! 数据库 Nonce 条目
//!
//! 定义了存储在数据库中的 Nonce 条目结构

use serde::{Deserialize, Serialize};

/// Nonce entry stored in the database
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct DbNonceEntry {
    pub id: Option<i64>,
    pub nonce: String,
    pub context: Option<String>,
    pub expires_at: i64, // Unix timestamp
    pub created_at: i64, // Unix timestamp
}
