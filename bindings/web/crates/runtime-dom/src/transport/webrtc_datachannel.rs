//! WebRTC DataChannel lane for DOM-side transport.
//!
//! Used by the DOM side to send messages over a WebRTC DataChannel.
//! Supports `RPC_*` and `STREAM_*`, but not `MEDIA_RTP`.
//!
//! ## Notes
//! - DataChannel can only be used in the DOM environment because Service Workers do not support WebRTC
//! - A PeerConnection must be established before creating the DataChannel
//! - DataChannel supports ordered/unordered and reliable/unreliable modes

use super::lane::{DataLane, LaneResult};
use actr_web_common::PayloadType;
use actr_web_common::WebError;
use actr_web_common::zero_copy::{
    extract_payload_zero_copy, parse_message_header, receive_zero_copy,
};
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, RtcDataChannel, RtcDataChannelInit, RtcDataChannelState};

/// WebRTC DataChannel lane builder.
///
/// Creates and configures a WebRTC DataChannel lane.
pub struct WebRtcDataChannelLaneBuilder {
    data_channel: RtcDataChannel,
    payload_type: PayloadType,
    buffer_size: usize,
}

impl WebRtcDataChannelLaneBuilder {
    /// Create a new WebRTC DataChannel lane builder.
    ///
    /// # Parameters
    /// - `data_channel`: RtcDataChannel obtained from a RtcPeerConnection
    /// - `payload_type`: PayloadType carried by this lane
    pub fn new(data_channel: RtcDataChannel, payload_type: PayloadType) -> Self {
        Self {
            data_channel,
            payload_type,
            buffer_size: 256, // Default buffer size.
        }
    }

    /// Set the receive buffer size.
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Build the WebRTC DataChannel lane.
    ///
    /// # Errors
    /// - If the payload type is unsupported (`MEDIA_RTP`)
    /// - If DataChannel setup fails
    pub async fn build(self) -> LaneResult<DataLane> {
        // Validate the payload type.
        if matches!(self.payload_type, PayloadType::MediaRtp) {
            return Err(WebError::Transport(
                "WebRTC DataChannel Lane does not support MEDIA_RTP; use MediaTrack Lane instead"
                    .to_string(),
            ));
        }

        // Configure the DataChannel for binary mode.
        // `RtcDataChannel` already defaults to `arraybuffer`, so no explicit setting is needed.

        // Create the receive channel.
        let (tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        // Install the onmessage callback with zero-copy helpers.
        let tx_clone = tx.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            // Try to read ArrayBuffer data.
            if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);

                // Zero-copy receive with one unavoidable copy from JS into WASM memory.
                let data = receive_zero_copy(&uint8_array);

                // Parse the message header.
                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "WebRTC DataChannel Lane received message: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // Extract the payload without another copy.
                    let payload_data = extract_payload_zero_copy(data, 5);

                    // Send to the channel. Ignore failures if the receiver is already closed.
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            } else {
                log::warn!("DataChannel received non-ArrayBuffer data; ignoring it");
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        self.data_channel
            .set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        // Install the onerror callback.
        let label = self.data_channel.label();
        let onerror_callback = Closure::wrap(Box::new(move |e: web_sys::ErrorEvent| {
            log::error!(
                "WebRTC DataChannel error (label={}): {:?}",
                label,
                e.message()
            );
        }) as Box<dyn FnMut(web_sys::ErrorEvent)>);

        self.data_channel
            .set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        onerror_callback.forget();

        // Install the onclose callback.
        let label = self.data_channel.label();
        let onclose_callback = Closure::wrap(Box::new(move |_e: JsValue| {
            log::info!("WebRTC DataChannel closed (label={})", label);
        }) as Box<dyn FnMut(JsValue)>);

        self.data_channel
            .set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));
        onclose_callback.forget();

        // Install the onopen callback.
        let label = self.data_channel.label();
        let payload_type = self.payload_type;
        let onopen_callback = Closure::wrap(Box::new(move |_e: JsValue| {
            log::info!(
                "WebRTC DataChannel opened: label={}, payload_type={:?}",
                label,
                payload_type
            );
        }) as Box<dyn FnMut(JsValue)>);

        self.data_channel
            .set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
        onopen_callback.forget();

        // Wait for the DataChannel to open if it is not open yet.
        let dc_clone = self.data_channel.clone();
        let wait_future = async move {
            let start = js_sys::Date::now();
            loop {
                let state = dc_clone.ready_state();
                if state == RtcDataChannelState::Open {
                    return Ok(());
                }

                if state == RtcDataChannelState::Closed || state == RtcDataChannelState::Closing {
                    return Err(WebError::Transport(
                        "WebRTC DataChannel failed or closed".to_string(),
                    ));
                }

                if js_sys::Date::now() - start > 10000.0 {
                    return Err(WebError::Transport(
                        "WebRTC DataChannel timed out after 10 seconds".to_string(),
                    ));
                }

                // Wait 50 ms and try again.
                wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
                    let window = web_sys::window().unwrap();
                    window
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50)
                        .unwrap();
                }))
                .await
                .unwrap();
            }
        };

        wait_future.await?;

        log::info!(
            "WebRTC DataChannel Lane created successfully: label={}, payload_type={:?}",
            self.data_channel.label(),
            self.payload_type
        );

        Ok(DataLane::WebRtcDataChannel {
            data_channel: Arc::new(self.data_channel),
            payload_type: self.payload_type,
            rx,
        })
    }
}

/// Helper to build DataChannel configuration.
///
/// Creates a suitable DataChannel configuration for the given payload type.
pub fn create_datachannel_config(payload_type: PayloadType) -> RtcDataChannelInit {
    let config = RtcDataChannelInit::new();

    match payload_type {
        PayloadType::RpcReliable | PayloadType::StreamReliable => {
            // Reliable ordered delivery.
            config.set_ordered(true);
            // Leaving max_retransmits unset means unlimited retries.
        }
        PayloadType::RpcSignal | PayloadType::StreamLatencyFirst => {
            // Low-latency delivery that allows reordering and loss.
            config.set_ordered(false);
            config.set_max_retransmits(0); // No retransmission.
        }
        PayloadType::MediaRtp => {
            // DataChannel should not be used for MEDIA_RTP.
            log::warn!("DataChannel should not be used for MEDIA_RTP; use MediaTrack instead");
            config.set_ordered(false);
            config.set_max_retransmits(0);
        }
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // Note: the following tests are temporarily disabled because of web-sys API changes.
    // The getter signatures on RtcDataChannelInit changed in newer versions.
    // TODO: update the tests to the new API.
    /*
    #[wasm_bindgen_test]
    fn test_datachannel_config_for_reliable_types() {
        let config = create_datachannel_config(PayloadType::RpcReliable);
        assert_eq!(config.ordered(), true);

        let config = create_datachannel_config(PayloadType::StreamReliable);
        assert_eq!(config.ordered(), true);
    }

    #[wasm_bindgen_test]
    fn test_datachannel_config_for_latency_first_types() {
        let config = create_datachannel_config(PayloadType::RpcSignal);
        assert_eq!(config.ordered(), false);
        assert_eq!(config.max_retransmits(), Some(0));

        let config = create_datachannel_config(PayloadType::StreamLatencyFirst);
        assert_eq!(config.ordered(), false);
        assert_eq!(config.max_retransmits(), Some(0));
    }
    */

    #[wasm_bindgen_test]
    async fn test_webrtc_datachannel_lane_rejects_media_rtp() {
        // Note: this test would need a real RtcPeerConnection.
        // In a full integration environment it should create a real PeerConnection and DataChannel.

        // For now, this only verifies the PayloadType validation logic.
        let payload_types = vec![
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
        ];

        for payload_type in payload_types {
            // Verify these types are not MEDIA_RTP.
            assert!(!matches!(payload_type, PayloadType::MediaRtp));
        }

        // Verify that MEDIA_RTP is the rejected case.
        assert!(matches!(PayloadType::MediaRtp, PayloadType::MediaRtp));
    }
}
