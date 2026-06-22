//! Actor Identity 管理模块
//!
//! 基于 aid proto 提供 Actor Identity Token 验证功能（签发功能已移至 ais crate）

pub mod credential;
pub mod identity_claims;
pub mod key_cache;

pub use credential::{AIdCredential, AIdCredentialValidator, AidError};
pub use identity_claims::IdentityClaims;
pub use key_cache::KeyCache;
