//! DOM-side inbound message dispatcher.
//!
//! Receives Fast Path messages from the Service Worker (`STREAM_*` / `MEDIA_RTP`)
//! and dispatches them to the corresponding registries.

use actr_web_common::{MessageFormat, PayloadType, WebError, WebResult};
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::fastpath::{MediaFrameHandlerRegistry, StreamHandlerRegistry};
use crate::transport::DataLane;

/// DOM-side inbound message dispatcher.
pub struct DomInboundDispatcher {
    /// Stream handler registry.
    stream_registry: Arc<StreamHandlerRegistry>,

    /// Media handler registry.
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// Communication lane to the Service Worker.
    sw_lane: Arc<Mutex<Option<DataLane>>>,
}

impl DomInboundDispatcher {
    /// Create a new dispatcher.
    pub fn new(
        stream_registry: Arc<StreamHandlerRegistry>,
        media_registry: Arc<MediaFrameHandlerRegistry>,
    ) -> Self {
        Self {
            stream_registry,
            media_registry,
            sw_lane: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the Service Worker communication lane.
    pub fn set_sw_lane(&self, lane: DataLane) {
        let mut sw_lane = self.sw_lane.lock();
        *sw_lane = Some(lane);
        log::info!("[DomInboundDispatcher] SW lane set");
    }

    /// Dispatch a received message.
    ///
    /// # Parameters
    /// - `data`: Raw message bytes
    pub fn dispatch(&self, data: Bytes) -> WebResult<()> {
        // Parse MessageFormat.
        let message = MessageFormat::try_from(data)?;

        match message.payload_type {
            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                self.dispatch_to_stream_registry(message)
            }
            PayloadType::MediaRtp => self.dispatch_to_media_registry(message),
            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                // RPC messages should not arrive at the DOM; they belong in the SW.
                log::warn!(
                    "[DomInboundDispatcher] Received RPC message in DOM, \
                     this should be handled in SW"
                );
                Err(WebError::Protocol(
                    "RPC messages should not arrive at DOM".to_string(),
                ))
            }
        }
    }

    /// Dispatch to StreamHandlerRegistry.
    fn dispatch_to_stream_registry(&self, message: MessageFormat) -> WebResult<()> {
        // Parse stream_id using a simplified protocol:
        // [stream_id_len(4) | stream_id(N) | chunk_data(M)]
        let data = message.data;
        if data.len() < 4 {
            return Err(WebError::Protocol(
                "Invalid stream message format".to_string(),
            ));
        }

        let stream_id_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + stream_id_len {
            return Err(WebError::Protocol(
                "Invalid stream message format".to_string(),
            ));
        }

        let stream_id = String::from_utf8(data[4..4 + stream_id_len].to_vec())
            .map_err(|e| WebError::Protocol(format!("Invalid stream_id: {}", e)))?;

        let chunk_data = data.slice(4 + stream_id_len..);

        // Dispatch to the registry.
        self.stream_registry.dispatch(&stream_id, chunk_data);

        log::debug!(
            "[DomInboundDispatcher] Stream message dispatched: stream_id={}",
            stream_id
        );

        Ok(())
    }

    /// Dispatch to MediaFrameRegistry.
    fn dispatch_to_media_registry(&self, message: MessageFormat) -> WebResult<()> {
        // Parse track_id using a simplified protocol:
        // [track_id_len(4) | track_id(N) | frame_data(M)]
        let data = message.data;
        if data.len() < 4 {
            return Err(WebError::Protocol(
                "Invalid media message format".to_string(),
            ));
        }

        let track_id_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + track_id_len {
            return Err(WebError::Protocol(
                "Invalid media message format".to_string(),
            ));
        }

        let track_id = String::from_utf8(data[4..4 + track_id_len].to_vec())
            .map_err(|e| WebError::Protocol(format!("Invalid track_id: {}", e)))?;

        let frame_data = data.slice(4 + track_id_len..);

        // Dispatch to the registry.
        self.media_registry.dispatch(&track_id, frame_data);

        log::debug!(
            "[DomInboundDispatcher] Media frame dispatched: track_id={}",
            track_id
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_dispatcher_creation() {
        let stream_registry = Arc::new(StreamHandlerRegistry::new());
        let media_registry = Arc::new(MediaFrameHandlerRegistry::new());
        let _dispatcher = DomInboundDispatcher::new(stream_registry, media_registry);
    }

    #[wasm_bindgen_test]
    fn test_dispatch_stream_message() {
        let stream_registry = Arc::new(StreamHandlerRegistry::new());
        let media_registry = Arc::new(MediaFrameHandlerRegistry::new());
        let dispatcher = DomInboundDispatcher::new(stream_registry.clone(), media_registry);

        // Register a test handler.
        let received = Arc::new(parking_lot::Mutex::new(false));
        let received_clone = received.clone();
        stream_registry.register(
            "test-stream".to_string(),
            Arc::new(move |_data| {
                *received_clone.lock() = true;
            }),
        );

        // Build a test message.
        // [stream_id_len(4) | stream_id(11="test-stream") | chunk_data(10="test-chunk")]
        let stream_id = b"test-stream";
        let chunk_data = b"test-chunk";
        let mut data = Vec::new();
        data.extend_from_slice(&(stream_id.len() as u32).to_be_bytes());
        data.extend_from_slice(stream_id);
        data.extend_from_slice(chunk_data);

        let message = MessageFormat::new(PayloadType::StreamReliable, Bytes::from(data));

        dispatcher
            .dispatch(message.to_bytes())
            .expect("Dispatch failed");

        // Verify the callback was invoked.
        assert!(*received.lock());
    }
}
