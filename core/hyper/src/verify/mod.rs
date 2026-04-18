//! Package verification module.
//!
//! Verifies `.actr` ZIP STORE packages via a pluggable [`TrustProvider`].

pub mod manifest;

#[cfg(not(target_arch = "wasm32"))]
pub mod cert_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod trust;

#[cfg(not(target_arch = "wasm32"))]
pub use cert_cache::MfrCertCache;
pub use manifest::PackageManifest;
#[cfg(not(target_arch = "wasm32"))]
pub use trust::{ChainTrust, RegistryTrust, StaticTrust, TrustProvider, verify_ed25519_manifest};
