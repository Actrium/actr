//! WebRTC DataChannel message receiver.
//!
//! Handles messages received on the DOM-side WebRTC DataChannel.

use actr_web_common::{MessageFormat, PayloadType, WebError, WebResult};
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, RtcDataChannel};

use crate::fastpath::{MediaFrameHandlerRegistry, StreamHandlerRegistry};
use crate::transport::DataLane;

/// WebRTC DataChannel message receiver.
///
/// Handles messages received over a WebRTC DataChannel:
/// - RPC messages: forwarded to the SW mailbox
/// - Stream messages: dispatched locally to StreamHandlerRegistry
/// - Media messages: logged as warnings because they should use MediaTrack
pub struct WebRtcDataChannelReceiver {
    /// Stream handler registry.
    stream_registry: Arc<StreamHandlerRegistry>,

    /// Media handler registry, even though media should normally use a track.
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// Service Worker lane used to forward RPC messages.
    sw_lane: Arc<Mutex<Option<DataLane>>>,
}

impl WebRtcDataChannelReceiver {
    /// Create a new receiver.
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

    /// Set the Service Worker lane.
    pub fn set_sw_lane(&self, lane: DataLane) {
        let mut sw_lane = self.sw_lane.lock();
        *sw_lane = Some(lane);
        log::info!("[WebRtcDataChannelReceiver] SW lane set");
    }

    /// Attach to a DataChannel.
    ///
    /// Installs the `onmessage` callback.
    pub fn attach_to_datachannel(&self, datachannel: &RtcDataChannel) -> WebResult<()> {
        let stream_registry = self.stream_registry.clone();
        let media_registry = self.media_registry.clone();
        let sw_lane = self.sw_lane.clone();

        let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
            // Handle the incoming message.
            if let Ok(array_buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);
                let data = uint8_array.to_vec();

                // Parse MessageFormat.
                match MessageFormat::try_from(data.as_slice()) {
                    Ok(message) => {
                        // Route by payload type.
                        match message.payload_type {
                            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                                // RPC messages: forward to the SW.
                                Self::forward_rpc_to_sw(&sw_lane, message);
                            }
                            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                                // Stream messages: dispatch locally.
                                if let Err(e) =
                                    Self::dispatch_stream_local(&stream_registry, message)
                                {
                                    log::error!(
                                        "[WebRtcDataChannelReceiver] Stream dispatch failed: {}",
                                        e
                                    );
                                }
                            }
                            PayloadType::MediaRtp => {
                                // Media messages: warn because they should use MediaTrack.
                                log::warn!(
                                    "[WebRtcDataChannelReceiver] Received MEDIA_RTP via DataChannel, \
                                     should use MediaTrack instead"
                                );
                                if let Err(e) = Self::dispatch_media_local(&media_registry, message)
                                {
                                    log::error!(
                                        "[WebRtcDataChannelReceiver] Media dispatch failed: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "[WebRtcDataChannelReceiver] Failed to parse MessageFormat: {}",
                            e
                        );
                    }
                }
            }
        }) as Box<dyn FnMut(_)>);

        datachannel.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        log::info!("[WebRtcDataChannelReceiver] Attached to DataChannel");
        Ok(())
    }

    /// Forward an RPC message to the Service Worker.
    fn forward_rpc_to_sw(sw_lane: &Arc<Mutex<Option<DataLane>>>, message: MessageFormat) {
        let sw_lane_guard = sw_lane.lock();

        if let Some(ref lane) = *sw_lane_guard {
            // Wrap and send to the SW.
            // The full format could be:
            // [MessageType(1) | From(serialized) | MessageFormat(serialized)]
            // For now, the simplified implementation forwards MessageFormat directly.
            let data = message.to_bytes();

            wasm_bindgen_futures::spawn_local({
                let lane = lane.clone();
                async move {
                    if let Err(e) = lane.send(data).await {
                        log::error!(
                            "[WebRtcDataChannelReceiver] Failed to forward RPC to SW: {}",
                            e
                        );
                    } else {
                        log::debug!(
                            "[WebRtcDataChannelReceiver] RPC message forwarded to SW: {:?}",
                            message.payload_type
                        );
                    }
                }
            });
        } else {
            log::warn!("[WebRtcDataChannelReceiver] SW lane not set, cannot forward RPC");
        }
    }

    /// Dispatch a stream message locally.
    fn dispatch_stream_local(
        stream_registry: &Arc<StreamHandlerRegistry>,
        message: MessageFormat,
    ) -> WebResult<()> {
        // Parse stream_id using:
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
        stream_registry.dispatch(&stream_id, chunk_data);

        log::debug!(
            "[WebRtcDataChannelReceiver] Stream message dispatched locally: stream_id={}",
            stream_id
        );

        Ok(())
    }

    /// Dispatch a media message locally.
    fn dispatch_media_local(
        media_registry: &Arc<MediaFrameHandlerRegistry>,
        message: MessageFormat,
    ) -> WebResult<()> {
        // Parse track_id using:
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
        media_registry.dispatch(&track_id, frame_data);

        log::debug!(
            "[WebRtcDataChannelReceiver] Media frame dispatched locally: track_id={}",
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
    fn test_receiver_creation() {
        let stream_registry = Arc::new(StreamHandlerRegistry::new());
        let media_registry = Arc::new(MediaFrameHandlerRegistry::new());
        let _receiver = WebRtcDataChannelReceiver::new(stream_registry, media_registry);
    }
}
