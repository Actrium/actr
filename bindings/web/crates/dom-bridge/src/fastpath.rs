//! Fast Path Registry
//!
//! DOM-side Fast Path registry for managing fast-dispatch callbacks for stream data and media frames.
//!
//! Fast Path mechanism:
//! - Stream data (STREAM_*) and media frames (MEDIA_RTP) bypass the Mailbox
//! - Dispatched directly to pre-registered callback functions
//! - Executed concurrently on I/O threads, no need to wait for Scheduler scheduling
//! - Latency ~100µs (compared to ~10-20ms for the State Path)

// TODO: Phase 2 implementation

use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;

/// Stream handler callback type.
pub type StreamCallback = Arc<dyn Fn(Bytes) + Send + Sync>;

/// Media frame handler callback type.
pub type MediaFrameCallback = Arc<dyn Fn(Bytes) + Send + Sync>;

/// Stream handler registry.
///
/// Manages fast-dispatch callbacks for `STREAM_*` payloads.
pub struct StreamHandlerRegistry {
    handlers: DashMap<String, StreamCallback>,

    /// Clear callback used when the DOM runtime restarts.
    on_cleared: parking_lot::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl StreamHandlerRegistry {
    /// Create a new registry.
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
            on_cleared: parking_lot::Mutex::new(None),
        }
    }

    /// Register a stream handler callback.
    ///
    /// # Parameters
    /// - `stream_id`: Stream ID
    /// - `callback`: Handler callback
    pub fn register(&self, stream_id: String, callback: StreamCallback) {
        self.handlers.insert(stream_id.clone(), callback);
        log::debug!("Stream handler registered: stream_id={}", stream_id);
    }

    /// Unregister a stream handler callback.
    pub fn unregister(&self, stream_id: &str) {
        self.handlers.remove(stream_id);
        log::debug!("Stream handler unregistered: stream_id={}", stream_id);
    }

    /// Dispatch stream data to the callback.
    pub fn dispatch(&self, stream_id: &str, data: Bytes) {
        if let Some(handler) = self.handlers.get(stream_id) {
            (handler.value())(data);
        } else {
            log::warn!("No handler found for stream_id={}", stream_id);
        }
    }

    /// Set the clear callback.
    ///
    /// Called when the registry is cleared, for example after a DOM restart.
    pub fn on_cleared<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut on_cleared = self.on_cleared.lock();
        *on_cleared = Some(Arc::new(callback));
        log::debug!("Stream registry on_cleared callback registered");
    }

    /// Clear all handlers.
    ///
    /// Used during DOM restart to drop all registered callbacks.
    pub fn clear_all(&self) {
        let count = self.handlers.len();
        self.handlers.clear();

        log::warn!("[StreamRegistry] All handlers cleared (count={})", count);

        // Notify the user.
        if let Some(callback) = self.on_cleared.lock().as_ref() {
            callback();
        }
    }

    /// Export the current registration state.
    ///
    /// Returns all registered stream IDs.
    pub fn export_state(&self) -> Vec<String> {
        self.handlers
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Return the number of registered handlers.
    pub fn count(&self) -> usize {
        self.handlers.len()
    }
}

impl Default for StreamHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Media frame handler registry.
///
/// Manages fast-dispatch callbacks for `MEDIA_RTP` payloads.
pub struct MediaFrameHandlerRegistry {
    handlers: DashMap<String, MediaFrameCallback>,

    /// Clear callback used when the DOM runtime restarts.
    on_cleared: parking_lot::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl MediaFrameHandlerRegistry {
    /// Create a new registry.
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
            on_cleared: parking_lot::Mutex::new(None),
        }
    }

    /// Register a media frame handler callback.
    ///
    /// # Parameters
    /// - `track_id`: Track ID
    /// - `callback`: Handler callback
    pub fn register(&self, track_id: String, callback: MediaFrameCallback) {
        self.handlers.insert(track_id.clone(), callback);
        log::debug!("Media frame handler registered: track_id={}", track_id);
    }

    /// Unregister a media frame handler callback.
    pub fn unregister(&self, track_id: &str) {
        self.handlers.remove(track_id);
        log::debug!("Media frame handler unregistered: track_id={}", track_id);
    }

    /// Dispatch a media frame to the callback.
    pub fn dispatch(&self, track_id: &str, frame: Bytes) {
        if let Some(handler) = self.handlers.get(track_id) {
            (handler.value())(frame);
        } else {
            log::warn!("No handler found for track_id={}", track_id);
        }
    }

    /// Set the clear callback.
    ///
    /// Called when the registry is cleared, for example after a DOM restart.
    pub fn on_cleared<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut on_cleared = self.on_cleared.lock();
        *on_cleared = Some(Arc::new(callback));
        log::debug!("Media registry on_cleared callback registered");
    }

    /// Clear all handlers.
    ///
    /// Used during DOM restart to drop all registered callbacks.
    pub fn clear_all(&self) {
        let count = self.handlers.len();
        self.handlers.clear();

        log::warn!("[MediaRegistry] All handlers cleared (count={})", count);

        // Notify the user.
        if let Some(callback) = self.on_cleared.lock().as_ref() {
            callback();
        }
    }

    /// Export the current registration state.
    ///
    /// Returns all registered track IDs.
    pub fn export_state(&self) -> Vec<String> {
        self.handlers
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Return the number of registered handlers.
    pub fn count(&self) -> usize {
        self.handlers.len()
    }
}

impl Default for MediaFrameHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
