//! # actr-platform-native
//!
//! Native platform provider for Actor-RTC.
//!
//! Implements `PlatformProvider` using:
//! - **Storage**: SQLite via `ActorStore`
//! - **Crypto**: ed25519-dalek + sha2
//! - **Filesystem**: tokio::fs
//!
//! Also implements `MonotonicClock` ([`NativeMonotonicClock`]) with a
//! suspend-excluding backend per platform: `std::time::Instant` on Unix and
//! Apple targets, `QueryUnbiasedInterruptTime` on Windows.

pub mod clock;
pub mod crypto;
pub mod platform;

pub use clock::NativeMonotonicClock;
pub use crypto::NativeCryptoProvider;
pub use platform::NativePlatformProvider;
