//! Data lanes for DOM-side transport.
//!
//! The DOM side includes:
//! - PostMessage lane for communication with the Service Worker
//! - WebRTC DataChannel lane for P2P data transport
//! - WebRTC MediaTrack lane for media transport

use actr_web_common::zero_copy::{
    construct_message_header, construct_message_zero_copy, send_with_transfer, send_zero_copy,
    should_use_transfer,
};
use actr_web_common::{PayloadType, WebError, WebResult};
use bytes::Bytes;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{MessagePort, RtcDataChannel};

/// Result type for DOM transport operations.
pub type LaneResult<T> = WebResult<T>;

/// DOM-side data transport lane.
#[derive(Clone)]
pub enum DataLane {
    /// PostMessage lane used to communicate with the Service Worker.
    ///
    /// Supports all payload types.
    PostMessage {
        port: Arc<MessagePort>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },

    /// WebRTC DataChannel Lane
    ///
    /// Supports `RPC_*` and `STREAM_*`, but not `MEDIA_RTP`.
    WebRtcDataChannel {
        data_channel: Arc<RtcDataChannel>,
        payload_type: PayloadType,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },

    /// WebRTC MediaTrack Lane
    ///
    /// Supports only `MEDIA_RTP`.
    WebRtcMediaTrack {
        track_id: String,
        rx: Arc<Mutex<mpsc::UnboundedReceiver<Bytes>>>,
    },
}

impl DataLane {
    /// Send a message with zero-copy helpers where possible.
    pub async fn send(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage {
                port, payload_type, ..
            } => {
                // Build the message with zero-copy helpers.
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // Send by creating a WASM memory view.
                let js_view = send_zero_copy(&msg);

                port.post_message(&js_view.into()).map_err(|e| {
                    WebError::Transport(format!("PostMessage send failed: {:?}", e))
                })?;

                log::trace!(
                    "PostMessage Lane (DOM) sent message: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );

                Ok(())
            }

            DataLane::WebRtcDataChannel {
                data_channel,
                payload_type,
                ..
            } => {
                use web_sys::RtcDataChannelState;

                // Check the DataChannel state.
                if data_channel.ready_state() != RtcDataChannelState::Open {
                    return Err(WebError::Transport(
                        "WebRTC DataChannel is not open".to_string(),
                    ));
                }

                // Build the message with zero-copy helpers.
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // Send the `Bytes` slice directly because the DataChannel API accepts `&[u8]`.
                data_channel.send_with_u8_array(&msg).map_err(|e| {
                    WebError::Transport(format!("DataChannel send failed: {:?}", e))
                })?;

                log::trace!(
                    "WebRTC DataChannel Lane sent message: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );

                Ok(())
            }

            DataLane::WebRtcMediaTrack { track_id, .. } => Err(WebError::Transport(format!(
                "MediaTrack Lane (track_id={}) does not support direct send",
                track_id
            ))),
        }
    }

    /// Send using Transferable Objects, supported only on PostMessage lanes.
    ///
    /// **Transferable Objects**:
    /// - Transfer ArrayBuffer ownership instead of copying
    /// - Useful for large payloads (>10 KB)
    /// - Supported only by PostMessage lanes
    ///
    /// # Parameters
    /// - `data`: Data to send
    ///
    /// # Returns
    /// - `Ok(())`: Send succeeded
    /// - `Err`: Send failed or this lane type does not support transfer
    ///
    /// # Recommendation
    /// - Use this for large payloads (>10 KB)
    /// - Use regular `send()` for small payloads
    /// - Or use `send_auto()` to choose automatically
    pub async fn send_with_transfer(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage {
                port, payload_type, ..
            } => {
                // Build the message with zero-copy helpers.
                let header = construct_message_header(*payload_type as u8, data.len());
                let msg = construct_message_zero_copy(&header, &data);

                // Send with Transferable Objects.
                let (js_view, transfer_list) = send_with_transfer(&msg);

                // Use the low-level wasm-bindgen API to call postMessage(message, transferList).
                let post_message_fn =
                    js_sys::Reflect::get(port.as_ref(), &JsValue::from_str("postMessage"))
                        .map_err(|e| {
                            WebError::Transport(format!("Failed to get postMessage: {:?}", e))
                        })?;

                let result = js_sys::Reflect::apply(
                    post_message_fn.unchecked_ref::<js_sys::Function>(),
                    port.as_ref(),
                    &js_sys::Array::of2(&js_view.into(), &transfer_list),
                );

                match result {
                    Ok(_) => {
                        log::trace!(
                            "PostMessage Lane (DOM) sent message with transfer: payload_type={:?}, size={} bytes",
                            payload_type,
                            data.len()
                        );
                        Ok(())
                    }
                    Err(e) => Err(WebError::Transport(format!(
                        "PostMessage with transfer failed: {:?}",
                        e
                    ))),
                }
            }

            DataLane::WebRtcDataChannel { .. } => {
                // DataChannel does not support Transferable Objects, so fall back to regular send.
                log::warn!(
                    "WebRTC DataChannel does not support Transferable Objects; falling back to regular send"
                );
                self.send(data).await
            }

            DataLane::WebRtcMediaTrack { track_id, .. } => Err(WebError::Transport(format!(
                "MediaTrack Lane (track_id={}) does not support send_with_transfer",
                track_id
            ))),
        }
    }

    /// Automatically choose a send strategy based on payload size.
    ///
    /// **Decision logic**:
    /// - PostMessage + data >= 10 KB -> `send_with_transfer()`
    /// - PostMessage + data < 10 KB -> regular `send()`
    /// - Other lanes -> regular `send()`
    ///
    /// # Parameters
    /// - `data`: Data to send
    ///
    /// # Returns
    /// - `Ok(())`: Send succeeded
    /// - `Err`: Send failed
    pub async fn send_auto(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage { .. } if should_use_transfer(data.len()) => {
                // Use Transferable Objects for large payloads.
                self.send_with_transfer(data).await
            }
            _ => {
                // Use regular send in all other cases.
                self.send(data).await
            }
        }
    }

    /// Receive a message.
    #[allow(clippy::await_holding_lock)] // wasm single-threaded: stream recv must hold lock across await
    pub async fn recv(&self) -> Option<Bytes> {
        use futures::StreamExt;

        match self {
            DataLane::PostMessage {
                rx, payload_type, ..
            } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "PostMessage Lane (DOM) received message: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );
                Some(data)
            }

            DataLane::WebRtcDataChannel {
                rx, payload_type, ..
            } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "WebRTC DataChannel Lane received message: payload_type={:?}, size={} bytes",
                    payload_type,
                    data.len()
                );
                Some(data)
            }

            DataLane::WebRtcMediaTrack { rx, track_id, .. } => {
                let mut rx_guard = rx.lock();
                let data = rx_guard.next().await?;
                log::trace!(
                    "WebRTC MediaTrack Lane received media frame: track_id={}, size={} bytes",
                    track_id,
                    data.len()
                );
                Some(data)
            }
        }
    }

    /// Return the payload type for this lane.
    pub fn payload_type(&self) -> PayloadType {
        match self {
            DataLane::PostMessage { payload_type, .. } => *payload_type,
            DataLane::WebRtcDataChannel { payload_type, .. } => *payload_type,
            DataLane::WebRtcMediaTrack { .. } => PayloadType::MediaRtp,
        }
    }
}
