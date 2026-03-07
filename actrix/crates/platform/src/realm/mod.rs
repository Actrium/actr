//! Realm 管理模块
//!
//! 提供多 Realm 管理功能，包括 Realm 配置、权限控制、数据迁移等

// 子模块
pub mod acl;
pub mod error;
pub mod model;
pub mod secret;
pub mod validation;

// 公共API导出
pub use acl::ActorAcl;
pub use error::RealmError;
pub use model::{Realm, RealmStatus};
pub use secret::{
    DEFAULT_REALM_SECRET_PREVIOUS_GRACE_SECS, REALM_SECRET_HEADER, RealmSecretCheck,
    RealmSecretRotation, RealmSecretState, get_realm_secret_state, hash_realm_secret,
    rotate_realm_secret, verify_realm_secret,
};
