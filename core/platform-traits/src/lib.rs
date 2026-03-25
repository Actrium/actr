//! # actr-platform-traits
//!
//! Platform abstraction traits for Actor-RTC.
//!
//! This crate defines the boundary between platform-agnostic orchestration logic
//! (Hyper, ActrNode runtime) and platform-specific implementations (native, web).
//!
//! ## Traits
//!
//! - [`KvStore`] — key-value storage (SQLite on native, IndexedDB on web)
//! - [`CryptoProvider`] — cryptographic primitives (ed25519-dalek on native, Web Crypto on web)
//! - [`PlatformProvider`] — composite provider grouping all platform services

pub mod crypto;
pub mod platform;
pub mod storage;

pub use crypto::CryptoProvider;
pub use platform::PlatformProvider;
pub use storage::{KvOp, KvStore, KvStoreClone};

use thiserror::Error;

/// Unified error type for platform operations
#[derive(Debug, Error)]
pub enum PlatformError {
    /// Storage operation failed
    #[error("storage error: {0}")]
    Storage(String),

    /// Cryptographic operation failed
    #[error("crypto error: {0}")]
    Crypto(String),

    /// I/O error
    #[error("io error: {0}")]
    Io(String),

    /// Other error
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for PlatformError {
    fn from(e: std::io::Error) -> Self {
        PlatformError::Io(e.to_string())
    }
}
