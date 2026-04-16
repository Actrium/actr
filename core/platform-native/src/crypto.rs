//! Native cryptography provider (ed25519-dalek + sha2)

use async_trait::async_trait;
use ed25519_dalek::{PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH, Signature, VerifyingKey};
use sha2::{Digest, Sha256};

use actr_platform_traits::{CryptoProvider, PlatformError};

/// Native cryptography provider backed by ed25519-dalek and sha2.
pub struct NativeCryptoProvider;

#[async_trait]
impl CryptoProvider for NativeCryptoProvider {
    async fn ed25519_verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), PlatformError> {
        let pubkey_bytes: [u8; PUBLIC_KEY_LENGTH] = public_key.try_into().map_err(|_| {
            PlatformError::Crypto(format!(
                "Ed25519 public key must be {PUBLIC_KEY_LENGTH} bytes"
            ))
        })?;

        let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
            .map_err(|e| PlatformError::Crypto(format!("invalid Ed25519 public key: {e}")))?;

        let sig_bytes: [u8; SIGNATURE_LENGTH] = signature.try_into().map_err(|_| {
            PlatformError::Crypto(format!(
                "Ed25519 signature must be {SIGNATURE_LENGTH} bytes"
            ))
        })?;
        let sig = Signature::from_bytes(&sig_bytes);

        verifying_key
            .verify_strict(message, &sig)
            .map_err(|e| PlatformError::Crypto(format!("Ed25519 verification failed: {e}")))
    }

    async fn sha256(&self, data: &[u8]) -> Result<[u8; 32], PlatformError> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        Ok(hasher.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    #[tokio::test]
    async fn sha256_produces_correct_hash() {
        let provider = NativeCryptoProvider;
        let hash = provider.sha256(b"hello").await.unwrap();
        // Known SHA-256 of "hello"
        let expected = [
            0x2c, 0xf2, 0x4d, 0xba, 0x5f, 0xb0, 0xa3, 0x0e, 0x26, 0xe8, 0x3b, 0x2a, 0xc5, 0xb9,
            0xe2, 0x9e, 0x1b, 0x16, 0x1e, 0x5c, 0x1f, 0xa7, 0x42, 0x5e, 0x73, 0x04, 0x33, 0x62,
            0x93, 0x8b, 0x98, 0x24,
        ];
        assert_eq!(hash, expected);
    }

    #[tokio::test]
    async fn ed25519_verify_valid_signature() {
        let provider = NativeCryptoProvider;
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let message = b"test message";
        let signature = signing_key.sign(message);

        let result = provider
            .ed25519_verify(verifying_key.as_bytes(), message, &signature.to_bytes())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ed25519_verify_rejects_bad_signature() {
        let provider = NativeCryptoProvider;
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let message = b"test message";
        let bad_sig = [0u8; 64];

        let result = provider
            .ed25519_verify(verifying_key.as_bytes(), message, &bad_sig)
            .await;
        assert!(result.is_err());
    }
}
