//! WebPlatformProvider — composite provider for browser environments.
//!
//! Wires `WebCryptoProvider` + `IndexedDbKvStore` into a single `PlatformProvider`.
//!
//! ## Instance UID persistence
//!
//! `instance_uid` first attempts `localStorage` (available in Window and some
//! Worker contexts), falling back to an IndexedDB-backed store when
//! `localStorage` is not accessible (e.g. Service Workers).

use std::sync::Arc;

use actr_platform_traits::{CryptoProvider, KvStore, PlatformError, PlatformProvider};
use async_trait::async_trait;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::crypto::WebCryptoProvider;
use crate::storage::IndexedDbKvStore;

/// Key prefix for instance UIDs stored in localStorage.
const INSTANCE_UID_LS_PREFIX: &str = "actr_instance_uid_";

/// Namespace for the IndexedDB fallback store used when localStorage is unavailable.
const INSTANCE_UID_IDB_NAMESPACE: &str = "__actr_instance";

/// Composite platform provider for browser environments.
///
/// Composes [`WebCryptoProvider`] for cryptographic operations and
/// [`IndexedDbKvStore`] for per-Actor key-value storage.
///
/// `root_namespace` acts as the web equivalent of `data_dir` on native —
/// it disambiguates multiple Actr hosts living in the same browser origin.
pub struct WebPlatformProvider {
    root_namespace: String,
    crypto: Arc<WebCryptoProvider>,
}

impl WebPlatformProvider {
    /// Create a new web platform provider rooted at the given namespace.
    pub fn new(root_namespace: impl Into<String>) -> Self {
        let root_namespace = root_namespace.into();
        debug!(root = %root_namespace, "WebPlatformProvider initialized");
        Self {
            root_namespace,
            crypto: Arc::new(WebCryptoProvider),
        }
    }
}

impl Default for WebPlatformProvider {
    fn default() -> Self {
        Self::new("default")
    }
}

#[async_trait(?Send)]
impl PlatformProvider for WebPlatformProvider {
    async fn instance_uid(&self) -> Result<String, PlatformError> {
        let ls_key = format!("{INSTANCE_UID_LS_PREFIX}{}", self.root_namespace);

        // Attempt 1: try localStorage (fast, synchronous)
        match try_local_storage_get(&ls_key) {
            Ok(Some(id)) if !id.is_empty() => {
                debug!(instance_uid = %id, "loaded instance_uid from localStorage");
                return Ok(id);
            }
            Ok(Some(_)) => {
                warn!("instance_uid in localStorage is empty; will regenerate");
            }
            Ok(None) => {
                // Not stored yet, fall through to creation
            }
            Err(e) => {
                // localStorage unavailable (likely Service Worker context)
                debug!(error = %e, "localStorage unavailable, falling back to IndexedDB");
                return self.instance_uid_idb().await;
            }
        }

        // Generate a new instance UID
        let new_id = Uuid::new_v4().to_string();

        // Store in localStorage
        match try_local_storage_set(&ls_key, &new_id) {
            Ok(()) => {
                info!(instance_uid = %new_id, "generated and stored new instance_uid in localStorage");
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "failed to write instance_uid to localStorage, falling back to IndexedDB"
                );
                return self.store_instance_uid_idb(&new_id).await;
            }
        }

        Ok(new_id)
    }

    async fn secret_store(&self, namespace: &str) -> Result<Arc<dyn KvStore>, PlatformError> {
        let store = IndexedDbKvStore::new(namespace);
        debug!(namespace = %namespace, "opened IndexedDB secret store");
        Ok(Arc::new(store))
    }

    fn crypto(&self) -> Arc<dyn CryptoProvider> {
        self.crypto.clone()
    }
}

// ---------------------------------------------------------------------------
// IndexedDB fallback for instance UID (Service Worker support)
// ---------------------------------------------------------------------------

impl WebPlatformProvider {
    /// Load or create instance UID using IndexedDB as fallback storage.
    async fn instance_uid_idb(&self) -> Result<String, PlatformError> {
        let store = IndexedDbKvStore::new(INSTANCE_UID_IDB_NAMESPACE);
        let idb_key = format!("instance_uid_{}", self.root_namespace);

        // Check if already stored
        if let Some(bytes) = store.get(&idb_key).await? {
            let id = String::from_utf8(bytes).map_err(|e| {
                PlatformError::Storage(format!("instance_uid is not valid UTF-8: {e}"))
            })?;
            if !id.is_empty() {
                debug!(instance_uid = %id, "loaded instance_uid from IndexedDB fallback");
                return Ok(id);
            }
            warn!("instance_uid in IndexedDB fallback is empty; will regenerate");
        }

        let new_id = Uuid::new_v4().to_string();
        self.store_instance_uid_idb(&new_id).await?;
        Ok(new_id)
    }

    /// Persist an instance UID to the IndexedDB fallback store.
    async fn store_instance_uid_idb(&self, instance_uid: &str) -> Result<String, PlatformError> {
        let store = IndexedDbKvStore::new(INSTANCE_UID_IDB_NAMESPACE);
        let idb_key = format!("instance_uid_{}", self.root_namespace);

        store.set(&idb_key, instance_uid.as_bytes()).await?;
        info!(
            instance_uid = %instance_uid,
            "generated and stored new instance_uid in IndexedDB fallback"
        );
        Ok(instance_uid.to_string())
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
