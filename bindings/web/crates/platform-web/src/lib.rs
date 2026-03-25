//! # actr-platform-web
//!
//! Web platform implementation for Actor-RTC.
//!
//! Provides browser-native implementations of the platform abstraction traits
//! defined in `actr-platform-traits`, enabling Hyper to run in Service Worker
//! and DOM environments.
//!
//! ## Modules
//!
//! - [`crypto`] — Ed25519 verify + SHA-256 via Web Crypto API (SubtleCrypto)
//! - [`storage`] — Key-value store backed by IndexedDB
//! - [`platform`] — Composite [`PlatformProvider`] wiring crypto + storage
//! - [`mailbox`] — Message queue backed by IndexedDB

pub mod crypto;
pub mod mailbox;
pub mod platform;
pub mod storage;

pub use crypto::WebCryptoProvider;
pub use mailbox::IndexedDbMailbox;
pub use platform::WebPlatformProvider;
pub use storage::IndexedDbKvStore;
