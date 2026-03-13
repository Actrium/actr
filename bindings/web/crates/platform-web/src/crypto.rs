//! Web Crypto API implementation of CryptoProvider
//!
//! Uses SubtleCrypto for Ed25519 verification and SHA-256 hashing.
//! Works in both Window and Worker (Service Worker) contexts.

use actr_platform_traits::{CryptoProvider, PlatformError};
use async_trait::async_trait;
use js_sys::{Object, Uint8Array};
use tracing::{debug, warn};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Zero-sized Web Crypto provider backed by SubtleCrypto
#[derive(Debug, Clone)]
pub struct WebCryptoProvider;

/// Obtain `SubtleCrypto` from the global scope.
///
/// Tries the generic `globalThis.crypto.subtle` path, which works in both
/// Window and WorkerGlobalScope (including Service Workers).
fn get_subtle_crypto() -> Result<web_sys::SubtleCrypto, PlatformError> {
    let global = js_sys::global();
    let crypto = js_sys::Reflect::get(&global, &"crypto".into())
        .map_err(|_| PlatformError::Crypto("crypto not available".into()))?;
    let subtle = js_sys::Reflect::get(&crypto, &"subtle".into())
        .map_err(|_| PlatformError::Crypto("subtle crypto not available".into()))?;
    Ok(web_sys::SubtleCrypto::from(subtle))
}

/// Convert a `JsValue` error to `PlatformError::Crypto`.
fn js_err(context: &str, e: JsValue) -> PlatformError {
    let msg = if let Some(err) = e.dyn_ref::<js_sys::Error>() {
        format!("{context}: {}", err.message())
    } else {
        format!("{context}: {e:?}")
    };
    PlatformError::Crypto(msg)
}

#[async_trait(?Send)]
impl CryptoProvider for WebCryptoProvider {
    async fn ed25519_verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), PlatformError> {
        debug!(
            pk_len = public_key.len(),
            msg_len = message.len(),
            sig_len = signature.len(),
            "ed25519_verify: starting verification via SubtleCrypto"
        );

        if public_key.len() != 32 {
            return Err(PlatformError::Crypto(format!(
                "invalid Ed25519 public key length: expected 32 bytes, got {}",
                public_key.len()
            )));
        }
        if signature.len() != 64 {
            return Err(PlatformError::Crypto(format!(
                "invalid Ed25519 signature length: expected 64 bytes, got {}",
                signature.len()
            )));
        }

        let subtle = get_subtle_crypto()?;

        // Build the algorithm identifier object: { name: "Ed25519" }
        let algorithm = Object::new();
        js_sys::Reflect::set(&algorithm, &"name".into(), &"Ed25519".into())
            .map_err(|e| js_err("failed to build algorithm object", e))?;

        // Import the raw public key
        let pk_array = Uint8Array::from(public_key);
        let import_promise = subtle
            .import_key_with_object(
                "raw",
                &pk_array,
                &algorithm,
                false,
                &js_sys::Array::of1(&"verify".into()),
            )
            .map_err(|e| {
                warn!("Ed25519 importKey failed — SubtleCrypto may not support Ed25519");
                js_err("Ed25519 importKey not supported by this browser", e)
            })?;

        let crypto_key = JsFuture::from(import_promise).await.map_err(|e| {
            warn!("Ed25519 importKey promise rejected");
            js_err("Ed25519 importKey rejected", e)
        })?;

        // Verify the signature
        let sig_array = Uint8Array::from(signature);
        let msg_array = Uint8Array::from(message);

        let verify_promise = subtle
            .verify_with_object_and_buffer_source_and_buffer_source(
                &algorithm,
                &web_sys::CryptoKey::from(crypto_key),
                &sig_array,
                &msg_array,
            )
            .map_err(|e| js_err("subtle.verify call failed", e))?;

        let result = JsFuture::from(verify_promise)
            .await
            .map_err(|e| js_err("subtle.verify promise rejected", e))?;

        let valid = result
            .as_bool()
            .ok_or_else(|| PlatformError::Crypto("verify did not return a boolean".into()))?;

        if valid {
            debug!("ed25519_verify: signature is valid");
            Ok(())
        } else {
            debug!("ed25519_verify: signature verification failed");
            Err(PlatformError::Crypto(
                "Ed25519 signature verification failed".into(),
            ))
        }
    }

    async fn sha256(&self, data: &[u8]) -> Result<[u8; 32], PlatformError> {
        debug!(len = data.len(), "sha256: hashing via SubtleCrypto");

        let subtle = get_subtle_crypto()?;

        let data_array = Uint8Array::from(data);
        let digest_promise = subtle
            .digest_with_str_and_buffer_source("SHA-256", &data_array)
            .map_err(|e| js_err("subtle.digest call failed", e))?;

        let array_buffer = JsFuture::from(digest_promise)
            .await
            .map_err(|e| js_err("subtle.digest promise rejected", e))?;

        let result_array = Uint8Array::new(&array_buffer);
        if result_array.length() != 32 {
            return Err(PlatformError::Crypto(format!(
                "SHA-256 digest returned {} bytes, expected 32",
                result_array.length()
            )));
        }

        let mut hash = [0u8; 32];
        result_array.copy_to(&mut hash);

        debug!("sha256: digest computed successfully");
        Ok(hash)
    }
}
