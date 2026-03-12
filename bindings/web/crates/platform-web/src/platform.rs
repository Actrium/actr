//! WebPlatformProvider — composite provider for browser environments
//!
//! Wires `WebCryptoProvider` + `IndexedDbKvStore` into a single `PlatformProvider`.
//!
//! ## Instance ID persistence
//!
//! `load_or_create_instance_id` first attempts `localStorage` (available in Window
//! and some Worker contexts), falling back to an IndexedDB-backed store when
//! `localStorage` is not accessible (e.g. Service Workers).

use std::sync::Arc;

use actr_platform_traits::{CryptoProvider, KvStore, PlatformError, PlatformProvider};
use async_trait::async_trait;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::crypto::WebCryptoProvider;
use crate::storage::IndexedDbKvStore;

/// Key prefix for instance IDs stored in localStorage.
const INSTANCE_ID_LS_PREFIX: &str = "actr_instance_id_";

/// Namespace for the IndexedDB fallback store used when localStorage is unavailable.
const INSTANCE_ID_IDB_NAMESPACE: &str = "__actr_instance";

/// Composite platform provider for browser environments.
///
/// Composes [`WebCryptoProvider`] for cryptographic operations and
/// [`IndexedDbKvStore`] for per-Actor key-value storage.
pub struct WebPlatformProvider {
    crypto: Arc<WebCryptoProvider>,
}

impl WebPlatformProvider {
    /// Create a new web platform provider.
    pub fn new() -> Self {
        debug!("WebPlatformProvider initialized");
        Self {
            crypto: Arc::new(WebCryptoProvider),
        }
    }
}

impl Default for WebPlatformProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl PlatformProvider for WebPlatformProvider {
    async fn open_kv_store(&self, namespace: &str) -> Result<Arc<dyn KvStore>, PlatformError> {
        let store = IndexedDbKvStore::new(namespace);
        debug!(namespace = %namespace, "opened IndexedDB KV store");
        Ok(Arc::new(store))
    }

    async fn load_or_create_instance_id(&self, data_dir: &str) -> Result<String, PlatformError> {
        let ls_key = format!("{INSTANCE_ID_LS_PREFIX}{data_dir}");

        // Attempt 1: try localStorage (fast, synchronous)
        match try_local_storage_get(&ls_key) {
            Ok(Some(id)) if !id.is_empty() => {
                debug!(instance_id = %id, "loaded instance_id from localStorage");
                return Ok(id);
            }
            Ok(Some(_)) => {
                warn!("instance_id in localStorage is empty; will regenerate");
            }
            Ok(None) => {
                // Not stored yet, fall through to creation
            }
            Err(e) => {
                // localStorage unavailable (likely Service Worker context)
                debug!(error = %e, "localStorage unavailable, falling back to IndexedDB");
                return self.load_or_create_instance_id_idb(data_dir).await;
            }
        }

        // Generate a new instance ID
        let new_id = Uuid::new_v4().to_string();

        // Store in localStorage
        match try_local_storage_set(&ls_key, &new_id) {
            Ok(()) => {
                info!(instance_id = %new_id, "generated and stored new instance_id in localStorage");
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "failed to write instance_id to localStorage, falling back to IndexedDB"
                );
                return self.store_instance_id_idb(data_dir, &new_id).await;
            }
        }

        Ok(new_id)
    }

    async fn ensure_dir(&self, _path: &str) -> Result<(), PlatformError> {
        // No-op on web — there is no filesystem directory concept
        Ok(())
    }

    fn crypto(&self) -> Arc<dyn CryptoProvider> {
        self.crypto.clone()
    }
}

// ---------------------------------------------------------------------------
// IndexedDB fallback for instance ID (Service Worker support)
// ---------------------------------------------------------------------------

impl WebPlatformProvider {
    /// Load or create instance ID using IndexedDB as fallback storage.
    async fn load_or_create_instance_id_idb(
        &self,
        data_dir: &str,
    ) -> Result<String, PlatformError> {
        let store = IndexedDbKvStore::new(INSTANCE_ID_IDB_NAMESPACE);
        let idb_key = format!("instance_id_{data_dir}");

        // Check if already stored
        if let Some(bytes) = store.get(&idb_key).await? {
            let id = String::from_utf8(bytes).map_err(|e| {
                PlatformError::Storage(format!("instance_id is not valid UTF-8: {e}"))
            })?;
            if !id.is_empty() {
                debug!(instance_id = %id, "loaded instance_id from IndexedDB fallback");
                return Ok(id);
            }
            warn!("instance_id in IndexedDB fallback is empty; will regenerate");
        }

        let new_id = Uuid::new_v4().to_string();
        self.store_instance_id_idb(data_dir, &new_id).await?;
        Ok(new_id)
    }

    /// Persist an instance ID to the IndexedDB fallback store.
    async fn store_instance_id_idb(
        &self,
        data_dir: &str,
        instance_id: &str,
    ) -> Result<String, PlatformError> {
        let store = IndexedDbKvStore::new(INSTANCE_ID_IDB_NAMESPACE);
        let idb_key = format!("instance_id_{data_dir}");

        store.set(&idb_key, instance_id.as_bytes()).await?;
        info!(
            instance_id = %instance_id,
            "generated and stored new instance_id in IndexedDB fallback"
        );
        Ok(instance_id.to_string())
    }
}

// ---------------------------------------------------------------------------
// localStorage helpers
// ---------------------------------------------------------------------------

/// Try to read a value from localStorage.
///
/// Returns `Err` if localStorage is not accessible (e.g. in a Service Worker),
/// `Ok(None)` if the key doesn't exist, or `Ok(Some(value))` on success.
fn try_local_storage_get(key: &str) -> Result<Option<String>, PlatformError> {
    let storage = get_local_storage()?;
    storage
        .get_item(key)
        .map_err(|e| PlatformError::Storage(format!("localStorage.getItem failed: {e:?}")))
}

/// Try to write a value to localStorage.
///
/// Returns `Err` if localStorage is not accessible or write fails (e.g. quota exceeded).
fn try_local_storage_set(key: &str, value: &str) -> Result<(), PlatformError> {
    let storage = get_local_storage()?;
    storage
        .set_item(key, value)
        .map_err(|e| PlatformError::Storage(format!("localStorage.setItem failed: {e:?}")))
}

/// Obtain a handle to `localStorage` from the global scope.
///
/// Works in Window contexts. Fails in Service Worker contexts where
/// `localStorage` is not available.
fn get_local_storage() -> Result<web_sys::Storage, PlatformError> {
    let global = js_sys::global();

    // Try Window.localStorage
    let storage_val = js_sys::Reflect::get(&global, &"localStorage".into())
        .map_err(|_| PlatformError::Storage("localStorage not accessible".into()))?;

    if storage_val.is_undefined() || storage_val.is_null() {
        return Err(PlatformError::Storage(
            "localStorage is null or undefined (likely Service Worker context)".into(),
        ));
    }

    Ok(web_sys::Storage::from(storage_val))
}
