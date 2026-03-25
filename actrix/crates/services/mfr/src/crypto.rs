use crate::MfrError;
use base64::Engine as _;
use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

/// Compute deterministic key_id from Ed25519 public key bytes.
///
/// Algorithm: `"mfr-" + hex(sha256(public_key_bytes))[..16]`
///
/// This MUST match the client-side implementation in `actr_pack::compute_key_id`.
pub fn compute_key_id(public_key_bytes: &[u8]) -> String {
    let hash = Sha256::digest(public_key_bytes);
    let hex_str: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("mfr-{}", &hex_str[..16])
}

/// Compute key_id from a base64-encoded Ed25519 public key string.
pub fn compute_key_id_from_b64(public_key_b64: &str) -> Result<String, MfrError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64)
        .map_err(|e| MfrError::Crypto(format!("invalid public key base64: {e}")))?;
    Ok(compute_key_id(&bytes))
}

/// Verify an Ed25519 signature over `message` using a base64-encoded public key.
pub fn verify_signature(
    message: &[u8],
    signature_b64: &str,
    public_key_b64: &str,
) -> Result<bool, MfrError> {
    let pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64)
        .map_err(|e| MfrError::Crypto(format!("invalid public key encoding: {e}")))?;
    let pubkey_arr: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| MfrError::Crypto("public key must be 32 bytes".to_string()))?;
    let verifying_key = VerifyingKey::from_bytes(&pubkey_arr)
        .map_err(|e| MfrError::Crypto(format!("invalid public key: {e}")))?;

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_b64)
        .map_err(|e| MfrError::Crypto(format!("invalid signature encoding: {e}")))?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| MfrError::Crypto("signature must be 64 bytes".to_string()))?;
    let signature = Signature::from_bytes(&sig_arr);

    Ok(verifying_key.verify(message, &signature).is_ok())
}

/// Generate a new Ed25519 keypair.
/// Returns (private_key_b64, public_key_b64).
pub fn generate_keypair() -> (String, String) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let private_b64 = base64::engine::general_purpose::STANDARD.encode(signing_key.to_bytes());
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());
    (private_b64, public_b64)
}

/// Validate that a base64-encoded string is a valid Ed25519 public key.
/// Returns Ok(()) if valid, or a descriptive Crypto error.
pub fn validate_public_key(public_key_b64: &str) -> Result<(), MfrError> {
    let pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64)
        .map_err(|e| MfrError::Crypto(format!("invalid public key encoding: {e}")))?;
    let pubkey_arr: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| MfrError::Crypto("public key must be 32 bytes".to_string()))?;
    VerifyingKey::from_bytes(&pubkey_arr)
        .map_err(|e| MfrError::Crypto(format!("invalid Ed25519 public key: {e}")))?;
    Ok(())
}
