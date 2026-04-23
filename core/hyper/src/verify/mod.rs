//! Package verification module.
//!
//! Verifies `.actr` ZIP STORE packages via a pluggable [`TrustProvider`].

#[cfg(not(target_arch = "wasm32"))]
mod cert_cache;
#[cfg(not(target_arch = "wasm32"))]
mod trust;

#[cfg(not(target_arch = "wasm32"))]
pub use cert_cache::MfrCertCache;
#[cfg(not(target_arch = "wasm32"))]
pub use trust::{ChainTrust, RegistryTrust, StaticTrust, TrustProvider};
