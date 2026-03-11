//! DOM System Module
//!
//! Runtime system for the DOM side.
//! Owns the Fast Path registries for stream handlers and media frame handlers.
//!
//! # SW <-> DOM connection
//!
//! `DomSystem` talks to the Service Worker via PostMessage:
//! - SW -> DOM: Fast Path data (`STREAM_*`, `MEDIA_RTP`)
//! - DOM -> SW: RPC forwarding

use crate::fastpath::{
    MediaFrameCallback, MediaFrameHandlerRegistry, StreamCallback, StreamHandlerRegistry,
};
use crate::transport::DataLane;
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;

/// DOM runtime system.
///
/// Manages the DOM-side Fast Path registries:
/// - Stream handler registry for stream data (`STREAM_*`)
/// - Media frame handler registry for media frames (`MEDIA_RTP`)
pub struct DomSystem {
    /// Stream handler registry.
    stream_registry: Arc<StreamHandlerRegistry>,

    /// Media frame handler registry.
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// Service Worker communication lane.
    sw_lane: Arc<Mutex<Option<DataLane>>>,
}

impl DomSystem {
    /// Create a new DOM system.
    pub fn new() -> Self {
        Self {
            stream_registry: Arc::new(StreamHandlerRegistry::new()),
            media_registry: Arc::new(MediaFrameHandlerRegistry::new()),
            sw_lane: Arc::new(Mutex::new(None)),
        }
    }

    // ========== Service Worker Connection Management ==========

    /// Set the Service Worker communication lane.
    ///
    /// Used to forward RPC messages to the Service Worker.
    pub fn set_sw_lane(&self, lane: DataLane) {
        let mut sw_lane = self.sw_lane.lock();
        *sw_lane = Some(lane);
        log::info!("[DomSystem] SW lane connected");
    }

    /// Send a message to the Service Worker.
    ///
    /// Used to forward RPC messages into the Service Worker mailbox.
    pub async fn send_to_sw(&self, data: Bytes) -> Result<(), String> {
        let sw_lane = self.sw_lane.lock();
        if let Some(ref lane) = *sw_lane {
            lane.send(data)
                .await
                .map_err(|e| format!("Failed to send to SW: {}", e))
        } else {
            Err("SW lane not connected".to_string())
        }
    }

    // ========== Stream Handler Management ==========

    /// Register a stream handler.
    ///
    /// # Parameters
    /// - `stream_id`: Stream ID
    /// - `callback`: Handler callback
    ///
    /// # Example
    /// ```ignore
    /// system.register_stream_handler("video_stream".to_string(), Arc::new(|data| {
    ///     // Process stream data
    /// }));
    /// ```
    pub fn register_stream_handler(&self, stream_id: String, callback: StreamCallback) {
        self.stream_registry.register(stream_id, callback);
    }

    /// Unregister a stream handler.
    pub fn unregister_stream_handler(&self, stream_id: &str) {
        self.stream_registry.unregister(stream_id);
    }

    /// Dispatch stream data.
    ///
    /// Called by the transport layer to dispatch received stream data.
    pub fn dispatch_stream(&self, stream_id: &str, data: Bytes) {
        self.stream_registry.dispatch(stream_id, data);
    }

    // ========== Media Handler Management ==========

    /// Register a media frame handler.
    ///
    /// # Parameters
    /// - `track_id`: Track ID
    /// - `callback`: Handler callback
    ///
    /// # Example
    /// ```ignore
    /// system.register_media_handler("audio_track".to_string(), Arc::new(|frame| {
    ///     // Process media frames
    /// }));
    /// ```
    pub fn register_media_handler(&self, track_id: String, callback: MediaFrameCallback) {
        self.media_registry.register(track_id, callback);
    }

    /// Unregister a media frame handler.
    pub fn unregister_media_handler(&self, track_id: &str) {
        self.media_registry.unregister(track_id);
    }

    /// Dispatch a media frame.
    ///
    /// Called by the transport layer to dispatch received media frames.
    pub fn dispatch_media_frame(&self, track_id: &str, frame: Bytes) {
        self.media_registry.dispatch(track_id, frame);
    }

    // ========== Registry Accessors ==========

    /// Get the stream handler registry.
    ///
    /// Used by the transport layer.
    pub fn stream_registry(&self) -> Arc<StreamHandlerRegistry> {
        Arc::clone(&self.stream_registry)
    }

    /// Get the media frame handler registry.
    ///
    /// Used by the transport layer.
    pub fn media_registry(&self) -> Arc<MediaFrameHandlerRegistry> {
        Arc::clone(&self.media_registry)
    }
}

impl Default for DomSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dom_system_creation() {
        let _system = DomSystem::new();
    }

    #[test]
    fn test_stream_handler_registration() {
        let system = DomSystem::new();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = Arc::clone(&called);

        system.register_stream_handler(
            "test_stream".to_string(),
            Arc::new(move |_data| {
                called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );

        system.dispatch_stream("test_stream", Bytes::from("test"));
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_media_handler_registration() {
        let system = DomSystem::new();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = Arc::clone(&called);

        system.register_media_handler(
            "test_track".to_string(),
            Arc::new(move |_frame| {
                called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );

        system.dispatch_media_frame("test_track", Bytes::from("test"));
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
