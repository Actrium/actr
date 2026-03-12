//! # actr-runtime-native
//!
//! Native platform provider for Actor-RTC.
//!
//! Implements `PlatformProvider` using:
//! - **Storage**: SQLite via `ActorStore`
//! - **Crypto**: ed25519-dalek + sha2
//! - **Filesystem**: tokio::fs

pub mod crypto;
pub mod platform;

pub use crypto::NativeCryptoProvider;
pub use platform::NativePlatformProvider;
