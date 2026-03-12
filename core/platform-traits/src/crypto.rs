//! Platform-agnostic cryptography provider trait

use async_trait::async_trait;

use crate::PlatformError;

/// Platform-agnostic cryptography provider
///
/// Native: ed25519-dalek + sha2
/// Web: SubtleCrypto (Web Crypto API)
#[async_trait]
pub trait CryptoProvider: Send + Sync {
    /// Ed25519 signature verification
    ///
    /// Returns `Ok(())` if the signature is valid, `Err` otherwise.
    async fn ed25519_verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), PlatformError>;

    /// SHA-256 hash
    async fn sha256(&self, data: &[u8]) -> Result<[u8; 32], PlatformError>;
}
