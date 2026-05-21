//! PostMessage lane for the DOM side (receives messages from the Service Worker).
//!
//! DOM-side PostMessage lane used to receive messages from the Service Worker.

use super::lane::{DataLane, LaneResult};
use actr_web_common::PayloadType;
use actr_web_common::zero_copy::{
    extract_payload_zero_copy, parse_message_header, receive_zero_copy,
};
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, MessagePort};

/// PostMessage lane builder for the DOM side.
pub struct PostMessageLaneBuilder {
    port: MessagePort,
    payload_type: PayloadType,
    buffer_size: usize,
}

impl PostMessageLaneBuilder {
    /// Create a new PostMessage lane builder.
    ///
    /// # Parameters
    /// - `port`: A MessagePort, typically one end of a MessageChannel
    /// - `payload_type`: PayloadType carried by this lane
    pub fn new(port: MessagePort, payload_type: PayloadType) -> Self {
        Self {
            port,
            payload_type,
            buffer_size: 256,
        }
    }

    /// Set the receive buffer size.
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Build the PostMessage lane.
    pub fn build(self) -> LaneResult<DataLane> {
        // Create the receive channel.
        let (tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        // Install the onmessage callback with zero-copy helpers.
        let tx_clone = tx.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            // Try to read Uint8Array data first.
            if let Ok(uint8_array) = e.data().dyn_into::<js_sys::Uint8Array>() {
                // Zero-copy receive with one unavoidable copy from JS memory into WASM memory.
                let data = receive_zero_copy(&uint8_array);

                // Parse the message header.
                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "PostMessage Lane (DOM) received message: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // Extract the payload by transferring Vec ownership into Bytes.
                    let payload_data = extract_payload_zero_copy(data, 5);
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            } else if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                // Also support ArrayBuffer payloads.
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);

                // Zero-copy receive.
                let data = receive_zero_copy(&uint8_array);

                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "PostMessage Lane (DOM) received message: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // Extract the payload without another copy.
                    let payload_data = extract_payload_zero_copy(data, 5);
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        self.port
            .set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        // Start the MessagePort.
        self.port.start();

        log::info!(
            "PostMessage Lane (DOM) created successfully: payload_type={:?}",
            self.payload_type
        );

        Ok(DataLane::PostMessage {
            port: Arc::new(self.port),
            payload_type: self.payload_type,
            rx,
        })
    }
}
