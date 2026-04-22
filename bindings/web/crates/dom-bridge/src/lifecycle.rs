//! DOM lifecycle management.
//!
//! Detects DOM-side startup, shutdown, and state changes, then notifies the
//! Service Worker.

use crate::{WebError, WebResult};
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Navigator, ServiceWorker, window};

/// DOM lifecycle manager.
pub struct DomLifecycleManager {
    /// Current DOM session ID.
    session_id: String,

    /// Whether initialization has already happened.
    initialized: Arc<Mutex<bool>>,
}

impl DomLifecycleManager {
    /// Create a new lifecycle manager.
    pub fn new() -> Self {
        let session_id = generate_session_id();
        log::info!("[DomLifecycle] Created new session: {}", session_id);

        Self {
            session_id,
            initialized: Arc::new(Mutex::new(false)),
        }
    }

    /// Return the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Initialize lifecycle management.
    ///
    /// Installs all event listeners and notifies the Service Worker of the current state.
    pub fn init(&self) -> WebResult<()> {
        let mut initialized = self.initialized.lock();
        if *initialized {
            log::warn!("[DomLifecycle] Already initialized");
            return Ok(());
        }

        log::info!("[DomLifecycle] Initializing lifecycle management");

        // 1. Listen for page load completion.
        self.setup_load_listener()?;

        // 2. Listen for the page about to unload.
        self.setup_beforeunload_listener()?;

        // 3. Listen for visibility changes.
        self.setup_visibility_listener()?;

        // 4. Immediately notify the SW with "DOM_READY" once the page is loaded.
        self.notify_dom_ready()?;

        *initialized = true;
        log::info!("[DomLifecycle] Lifecycle management initialized");

        Ok(())
    }

    /// Notify the SW that the DOM side is ready.
    fn notify_dom_ready(&self) -> WebResult<()> {
        log::info!("[DomLifecycle] Notifying SW: DOM_READY");

        let window = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let navigator = window.navigator();

        if let Some(controller) = get_sw_controller(&navigator) {
            let msg = create_lifecycle_message("DOM_READY", &self.session_id)?;
            controller
                .post_message(&msg)
                .map_err(|e| WebError::Internal(format!("Failed to post DOM_READY: {:?}", e)))?;

            log::info!("[DomLifecycle] DOM_READY sent successfully");
        } else {
            log::warn!("[DomLifecycle] No SW controller found, skipping DOM_READY");
        }

        Ok(())
    }

    /// Set up the load event listener.
    fn setup_load_listener(&self) -> WebResult<()> {
        let window = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move || {
            log::info!("[DomLifecycle] Page loaded, session_id={}", session_id);

            // The load-time notification already happens in init().
            // This listener is mainly for debugging and logging.
        }) as Box<dyn FnMut()>);

        window
            .add_event_listener_with_callback("load", callback.as_ref().unchecked_ref())
            .map_err(|e| WebError::Internal(format!("Failed to add load listener: {:?}", e)))?;

        callback.forget(); // Keep the listener alive.

        Ok(())
    }

    /// Set up the beforeunload event listener.
    fn setup_beforeunload_listener(&self) -> WebResult<()> {
        let win = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move |_event: web_sys::BeforeUnloadEvent| {
            log::info!("[DomLifecycle] Page unloading, session_id={}", session_id);

            // Notify the SW that the page is shutting down.
            if let Some(window_obj) = window() {
                let navigator = window_obj.navigator();

                if let Some(controller) = get_sw_controller(&navigator) {
                    if let Ok(msg) = create_lifecycle_message("DOM_UNLOADING", &session_id) {
                        // Best-effort send only; the page is about to close.
                        let _ = controller.post_message(&msg);

                        log::info!("[DomLifecycle] DOM_UNLOADING sent");
                    }
                }

                // sendBeacon is another option and is more reliable, but needs server support.
                // navigator.send_beacon_with_str(
                //     &format!("/api/lifecycle/unload?session={}", session_id),
                //     ""
                // ).ok();
            }
        }) as Box<dyn FnMut(web_sys::BeforeUnloadEvent)>);

        win.add_event_listener_with_callback("beforeunload", callback.as_ref().unchecked_ref())
            .map_err(|e| {
                WebError::Internal(format!("Failed to add beforeunload listener: {:?}", e))
            })?;

        callback.forget();

        Ok(())
    }

    /// Set up the visibilitychange event listener.
    fn setup_visibility_listener(&self) -> WebResult<()> {
        let win = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let document = win
            .document()
            .ok_or_else(|| WebError::Internal("No document".into()))?;
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move || {
            if let Some(window_obj) = window() {
                if let Some(doc) = window_obj.document() {
                    let hidden = doc.hidden();

                    log::debug!("[DomLifecycle] Visibility changed: hidden={}", hidden);

                    if !hidden {
                        // When the tab becomes visible, check whether the SW is still alive.
                        log::debug!("[DomLifecycle] Tab became visible, checking SW health");
                        check_sw_alive(&session_id);
                    }
                }
            }
        }) as Box<dyn FnMut()>);

        document
            .add_event_listener_with_callback("visibilitychange", callback.as_ref().unchecked_ref())
            .map_err(|e| {
                WebError::Internal(format!("Failed to add visibilitychange listener: {:?}", e))
            })?;

        callback.forget();

        Ok(())
    }
}

impl Default for DomLifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a unique session ID.
fn generate_session_id() -> String {
    use js_sys::{Date, Math};

    let timestamp = Date::now() as u64;
    let random = (Math::random() * 1_000_000.0) as u64;

    format!("dom-{}-{}", timestamp, random)
}

/// Get the Service Worker controller.
fn get_sw_controller(navigator: &Navigator) -> Option<ServiceWorker> {
    navigator.service_worker().controller()
}

/// Create a lifecycle message.
fn create_lifecycle_message(msg_type: &str, session_id: &str) -> WebResult<JsValue> {
    let msg = js_sys::Object::new();

    js_sys::Reflect::set(&msg, &"type".into(), &msg_type.into())
        .map_err(|e| WebError::Internal(format!("Failed to set type: {:?}", e)))?;

    js_sys::Reflect::set(&msg, &"session_id".into(), &session_id.into())
        .map_err(|e| WebError::Internal(format!("Failed to set session_id: {:?}", e)))?;

    Ok(msg.into())
}

/// Check whether the SW is alive.
///
/// Sends a ping message and expects a pong.
fn check_sw_alive(session_id: &str) {
    if let Some(window) = window() {
        let navigator = window.navigator();

        if let Some(controller) = get_sw_controller(&navigator) {
            if let Ok(msg) = create_lifecycle_message("DOM_PING", session_id) {
                match controller.post_message(&msg) {
                    Ok(_) => {
                        log::debug!("[DomLifecycle] SW ping sent");
                    }
                    Err(e) => {
                        log::warn!(
                            "[DomLifecycle] SW ping failed: {:?}, may need re-registration",
                            e
                        );
                        // TODO: Trigger Service Worker re-registration.
                    }
                }
            }
        } else {
            log::warn!("[DomLifecycle] No SW controller, may need re-registration");
            // TODO: Trigger Service Worker re-registration.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_generate_session_id() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();

        // Ensure different IDs are generated.
        assert_ne!(id1, id2);

        // Ensure the format is correct.
        assert!(id1.starts_with("dom-"));
        assert!(id2.starts_with("dom-"));
    }

    #[wasm_bindgen_test]
    fn test_lifecycle_manager_creation() {
        let manager = DomLifecycleManager::new();

        // Ensure a session_id exists.
        assert!(!manager.session_id().is_empty());
        assert!(manager.session_id().starts_with("dom-"));
    }
}
