//! Zero-copy utility module.
//!
//! Provides zero-copy, or near-zero-copy, helpers for WASM <-> JavaScript data transfer.
//!
//! ## Design principles
//!
//! 1. **Receive path optimization**
//!    - Avoid the double copy caused by `to_vec()` and `copy_from_slice()`
//!    - Transfer `Vec` ownership into `Bytes::from()` to avoid another copy
//!
//! 2. **Send path optimization**
//!    - Use `Uint8Array::view()` to create a WASM memory view without copying
//!    - Keep data lifetimes correct
//!
//! 3. **Memory safety**
//!    - Rely on Rust lifetimes where possible
//!    - Use `unsafe` carefully and keep bounds well-defined
//!
//! ## Example
//!
//! ```rust,no_run
//! use actr_web_common::zero_copy::{receive_zero_copy, send_zero_copy, extract_payload_zero_copy};
//! use bytes::Bytes;
//! use wasm_bindgen::prelude::*;
//! use wasm_bindgen::JsCast;
//!
//! // Mock port object
//! struct MockPort;
//! impl MockPort {
//!     fn post_message(&self, _msg: &JsValue) -> Result<(), JsValue> { Ok(()) }
//! }
//! let port = MockPort;
//!
//! // Receive path (JS -> Rust)
//! // Assume a Uint8Array was obtained from MessageEvent
//! let js_array = js_sys::Uint8Array::new_with_length(10);
//!
//! // Receive with one unavoidable copy from JS into WASM linear memory
//! let data = receive_zero_copy(&js_array);
//!
//! // Extract payload without another copy by transferring Vec ownership into Bytes
//! if data.len() >= 5 {
//!     let payload = extract_payload_zero_copy(data, 5);
//!     // ... use payload
//! }
//!
//! // Send path (Rust -> JS)
//! let data = Bytes::from(vec![1, 2, 3, 4, 5]);
//! // Zero-copy send by creating a WASM memory view
//! let js_view = send_zero_copy(&data);
//! port.post_message(&js_view.into()).unwrap();
//! ```

use bytes::Bytes;

/// Receive bytes from a JS Uint8Array into a Rust Vec.
///
/// **Performance notes**
/// - Old implementation: `uint8_array.to_vec()` copied the entire array
/// - New implementation: `copy_to()` writes directly into a preallocated Vec
///
/// **Copy count**: 1 copy from JS memory into WASM linear memory, which is unavoidable.
///
/// # Parameters
/// - `uint8_array`: JavaScript Uint8Array object
///
/// # Returns
/// - `Vec<u8>` allocated in WASM linear memory
///
/// # Notes
/// - This copy is required because JS and WASM use separate memory spaces
/// - It still avoids an extra copy compared to `to_vec()` plus `copy_from_slice()`
#[inline]
pub fn receive_zero_copy(uint8_array: &js_sys::Uint8Array) -> Vec<u8> {
    let len = uint8_array.length() as usize;
    // Initialize the buffer in a memory-safe way.
    let mut buffer = vec![0u8; len];

    // Use the browser-optimized memcpy path to write directly into WASM memory.
    uint8_array.copy_to(&mut buffer);

    buffer
}

/// Extract the payload from a Vec and transfer ownership into Bytes.
///
/// **Performance notes**
/// - Old implementation: `Bytes::copy_from_slice(&vec[offset..])` copied the payload again
/// - New implementation: `Vec::split_off()` plus `Bytes::from()` transfers ownership
///
/// **Copy count**: 0 additional copies because ownership is transferred.
///
/// # Parameters
/// - `mut data`: Vec containing the full message, consumed by this function
/// - `header_size`: Header length to skip
///
/// # Returns
/// - `Bytes` containing the payload
///
/// # Example
/// ```rust
/// use actr_web_common::zero_copy::extract_payload_zero_copy;
/// use bytes::Bytes;
///
/// let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];  // [header(5) | payload(5)]
/// let payload = extract_payload_zero_copy(data, 5);
/// // payload = Bytes([6, 7, 8, 9, 10])
/// ```
#[inline]
pub fn extract_payload_zero_copy(mut data: Vec<u8>, header_size: usize) -> Bytes {
    // split_off returns the back half (payload) and drops the front half (header).
    let payload_vec = data.split_off(header_size);

    // Bytes::from(Vec) transfers ownership with no extra copy.
    // Bytes uses Arc internally, so later clones also stay zero-copy.
    Bytes::from(payload_vec)
}

/// Create a JS Uint8Array view over Bytes without copying.
///
/// **Performance notes**
/// - Old implementation: `Uint8Array::from(&bytes[..])` copied the entire array
/// - New implementation: `Uint8Array::view()` creates a WASM memory view
///
/// **Copy count**: 0, because this only creates a view.
///
/// # Parameters
/// - `data`: Data to send
///
/// # Returns
/// - `js_sys::Uint8Array` view visible to JavaScript
///
/// # Safety
/// - The view must not outlive the backing data
/// - The view should be treated as read-only from JavaScript
/// - If Rust mutates the data, JavaScript will observe those changes
///
/// # Notes
/// - The backing `data` must remain alive while JS uses the view
/// - For PostMessage, browsers copy during `post_message`, so the view can be dropped immediately
/// - For WebSocket/DataChannel `send_with_u8_array`, copying is also synchronous
#[inline]
pub fn send_zero_copy(data: &Bytes) -> js_sys::Uint8Array {
    // Safe in practice because Uint8Array::view() creates a short-lived view and
    // browser messaging APIs copy the data immediately during the call.
    unsafe {
        let ptr = data.as_ptr();
        let len = data.len();
        js_sys::Uint8Array::view(std::slice::from_raw_parts(ptr, len))
    }
}

/// Build a message from header and payload with minimal copying.
///
/// **Approach**
/// - Preallocate a Vec with `header_size + payload_size`
/// - Write the header and payload into it
/// - Convert into Bytes
///
/// **Copy count**: 1 copy for the payload into the Vec, which is the practical minimum here.
///
/// # Parameters
/// - `header`: Header bytes
/// - `payload`: Payload bytes
///
/// # Returns
/// - `Bytes` containing the full message
#[inline]
pub fn construct_message_zero_copy(header: &[u8], payload: &Bytes) -> Bytes {
    let total_size = header.len() + payload.len();
    let mut msg = Vec::with_capacity(total_size);

    msg.extend_from_slice(header);
    msg.extend_from_slice(payload.as_ref());

    Bytes::from(msg)
}

/// Build a message while transferring ownership of the payload Vec.
///
/// **Optimization**
/// - If the payload is already a Vec, append it after the header directly
/// - Avoid the extra copy from `extend_from_slice`
///
/// **Copy count**: 0 extra copies during the append step.
///
/// # Parameters
/// - `header`: Header bytes
/// - `payload`: Payload Vec, consumed by this function
///
/// # Returns
/// - `Bytes` containing the full message
///
/// # Note
/// - This consumes `payload`, so use it only when the original payload does not need to be preserved
#[inline]
pub fn construct_message_from_vec(header: &[u8], mut payload: Vec<u8>) -> Bytes {
    // Insert the header before the payload.
    let mut msg = Vec::with_capacity(header.len() + payload.len());
    msg.extend_from_slice(header);
    msg.append(&mut payload); // append transfers ownership without another copy

    Bytes::from(msg)
}

/// Parse a message with format `[PayloadType(1) | Length(4) | Data(N)]`.
///
/// Returns `(payload_type, length, data_start_offset)`.
///
/// # Parameters
/// - `buffer`: Complete message buffer
///
/// # Returns
/// - `Some((payload_type, length, 5))` on success
/// - `None` if the message format is invalid or too short
#[inline]
pub fn parse_message_header(buffer: &[u8]) -> Option<(u8, usize, usize)> {
    if buffer.len() < 5 {
        return None;
    }

    let payload_type = buffer[0];
    let length = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;

    // Validate the message length.
    if buffer.len() < 5 + length {
        log::warn!(
            "Message length mismatch: expected {} bytes (header 5 + payload {}), got {} bytes",
            5 + length,
            length,
            buffer.len()
        );
        return None;
    }

    Some((payload_type, length, 5))
}

/// Build a message header in the format `[PayloadType(1) | Length(4)]`.
///
/// # Parameters
/// - `payload_type`: Numeric PayloadType value
/// - `payload_len`: Payload length
///
/// # Returns
/// - `[u8; 5]` header bytes
#[inline]
pub fn construct_message_header(payload_type: u8, payload_len: usize) -> [u8; 5] {
    let mut header = [0u8; 5];
    header[0] = payload_type;
    header[1..5].copy_from_slice(&(payload_len as u32).to_be_bytes());
    header
}

/// Create Transferable Object payloads for PostMessage.
///
/// **How Transferable Objects work**
/// - `postMessage(message, [transferList])` transfers ArrayBuffer ownership instead of copying
/// - The original buffer becomes detached after transfer
/// - The receiver gains exclusive ownership
/// - This is well-suited for large payloads (>10 KB)
///
/// **Performance benefits**
/// - Avoid the structured-clone overhead of plain postMessage
/// - Large payloads benefit significantly
/// - For small payloads, transfer overhead may outweigh copying
///
/// # Parameters
/// - `data`: Data to send
///
/// # Returns
/// - `(js_sys::Uint8Array, js_sys::Array)`: `(data_view, transfer_list)`
///
/// # Example
/// ```rust,no_run
/// use actr_web_common::zero_copy::send_with_transfer;
/// use bytes::Bytes;
/// use wasm_bindgen::JsValue;
///
/// // Mock port object
/// struct MockPort;
/// impl MockPort {
///     fn post_message_with_transfer(&self, _msg: &JsValue, _transfer: &js_sys::Array) -> Result<(), JsValue> { Ok(()) }
/// }
/// let port = MockPort;
///
/// let data = Bytes::from(vec![0u8; 100_000]); // 100 KB payload
/// let (js_view, transfer_list) = send_with_transfer(&data);
/// port.post_message_with_transfer(&js_view.into(), &transfer_list).unwrap();
/// ```
///
/// # Notes
/// - Only applicable to PostMessage, not WebSocket or DataChannel
/// - The original ArrayBuffer becomes detached after transfer
/// - Best for one-shot large payloads
#[inline]
pub fn send_with_transfer(data: &Bytes) -> (js_sys::Uint8Array, js_sys::Array) {
    // Create a WASM memory view.
    let js_view = unsafe {
        let ptr = data.as_ptr();
        let len = data.len();
        js_sys::Uint8Array::view(std::slice::from_raw_parts(ptr, len))
    };

    // Create the transfer list.
    let transfer_list = js_sys::Array::new();
    transfer_list.push(&js_view.buffer());

    (js_view, transfer_list)
}

/// Decide whether Transferable Objects should be used.
///
/// **Decision logic**
/// - Data size < 10 KB: not recommended
/// - Data size >= 10 KB: recommended
/// - Data size >= 100 KB: strongly recommended
///
/// # Parameters
/// - `data_size`: Data size in bytes
///
/// # Returns
/// - `true` when Transferable Objects are recommended
/// - `false` when regular send is fine
///
/// # Example
/// ```rust
/// use actr_web_common::zero_copy::should_use_transfer;
/// use bytes::Bytes;
///
/// let data = Bytes::from(vec![0u8; 50_000]);
/// if should_use_transfer(data.len()) {
///     // Use send_with_transfer()
/// } else {
///     // Use regular send()
/// }
/// ```
#[inline]
pub fn should_use_transfer(data_size: usize) -> bool {
    data_size >= 10 * 1024 // 10 KB threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // ===== receive_zero_copy tests =====

    #[wasm_bindgen_test]
    fn test_receive_zero_copy_basic() {
        let js_array = js_sys::Uint8Array::from(&[1u8, 2, 3, 4, 5][..]);
        let data = receive_zero_copy(&js_array);

        assert_eq!(data, vec![1, 2, 3, 4, 5]);
    }

    #[wasm_bindgen_test]
    fn test_receive_zero_copy_empty() {
        let js_array = js_sys::Uint8Array::new_with_length(0);
        let data = receive_zero_copy(&js_array);

        assert_eq!(data, Vec::<u8>::new());
    }

    #[wasm_bindgen_test]
    fn test_receive_zero_copy_large() {
        let large_data = vec![42u8; 1024 * 1024]; // 1MB
        let js_array = js_sys::Uint8Array::from(&large_data[..]);
        let data = receive_zero_copy(&js_array);

        assert_eq!(data.len(), 1024 * 1024);
        assert_eq!(data[0], 42);
        assert_eq!(data[1024 * 1024 - 1], 42);
    }

    // ===== extract_payload_zero_copy tests =====

    #[wasm_bindgen_test]
    fn test_extract_payload_zero_copy() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let payload = extract_payload_zero_copy(data, 5);

        assert_eq!(payload.as_ref(), &[6, 7, 8, 9, 10]);
    }

    #[wasm_bindgen_test]
    fn test_extract_payload_zero_copy_empty_payload() {
        let data = vec![1, 2, 3, 4, 5];
        let payload = extract_payload_zero_copy(data, 5);

        let empty: &[u8] = &[];
        assert_eq!(payload.as_ref(), empty);
    }

    #[wasm_bindgen_test]
    fn test_extract_payload_zero_copy_no_header() {
        let data = vec![1, 2, 3, 4, 5];
        let payload = extract_payload_zero_copy(data, 0);

        assert_eq!(payload.as_ref(), &[1, 2, 3, 4, 5]);
    }

    // ===== send_zero_copy tests =====

    #[wasm_bindgen_test]
    fn test_send_zero_copy_basic() {
        let data = Bytes::from_static(&[1, 2, 3, 4, 5]);
        let js_view = send_zero_copy(&data);

        assert_eq!(js_view.length(), 5);
        assert_eq!(js_view.get_index(0), 1);
        assert_eq!(js_view.get_index(4), 5);
    }

    #[wasm_bindgen_test]
    fn test_send_zero_copy_empty() {
        let data = Bytes::from_static(&[]);
        let js_view = send_zero_copy(&data);

        assert_eq!(js_view.length(), 0);
    }

    #[wasm_bindgen_test]
    fn test_send_zero_copy_large() {
        let large_data = Bytes::from(vec![42u8; 1024 * 1024]);
        let js_view = send_zero_copy(&large_data);

        assert_eq!(js_view.length(), 1024 * 1024);
        assert_eq!(js_view.get_index(0), 42);
        assert_eq!(js_view.get_index(1024 * 1024 - 1), 42);
    }

    // ===== construct_message_zero_copy tests =====

    #[wasm_bindgen_test]
    fn test_construct_message_zero_copy() {
        let header = [1, 0, 0, 0, 5]; // PayloadType=1, Length=5
        let payload = Bytes::from_static(&[10, 20, 30, 40, 50]);

        let msg = construct_message_zero_copy(&header, &payload);

        assert_eq!(msg.len(), 10);
        assert_eq!(&msg[0..5], &[1, 0, 0, 0, 5]);
        assert_eq!(&msg[5..10], &[10, 20, 30, 40, 50]);
    }

    // ===== construct_message_from_vec tests =====

    #[wasm_bindgen_test]
    fn test_construct_message_from_vec() {
        let header = [1, 0, 0, 0, 5];
        let payload = vec![10, 20, 30, 40, 50];

        let msg = construct_message_from_vec(&header, payload);

        assert_eq!(msg.len(), 10);
        assert_eq!(&msg[0..5], &[1, 0, 0, 0, 5]);
        assert_eq!(&msg[5..10], &[10, 20, 30, 40, 50]);
    }

    // ===== parse_message_header tests =====

    #[wasm_bindgen_test]
    fn test_parse_message_header_valid() {
        let buffer = [1, 0, 0, 0, 5, 10, 20, 30, 40, 50];

        let result = parse_message_header(&buffer);

        assert!(result.is_some());
        let (payload_type, length, offset) = result.unwrap();
        assert_eq!(payload_type, 1);
        assert_eq!(length, 5);
        assert_eq!(offset, 5);
    }

    #[wasm_bindgen_test]
    fn test_parse_message_header_too_short() {
        let buffer = [1, 0, 0, 0]; // Only 4 bytes.

        let result = parse_message_header(&buffer);

        assert!(result.is_none());
    }

    #[wasm_bindgen_test]
    fn test_parse_message_header_length_mismatch() {
        let buffer = [1, 0, 0, 0, 10, 1, 2, 3]; // Claims 10 bytes, actually only 3.

        let result = parse_message_header(&buffer);

        assert!(result.is_none());
    }

    // ===== construct_message_header tests =====

    #[wasm_bindgen_test]
    fn test_construct_message_header() {
        let header = construct_message_header(1, 5);

        assert_eq!(header, [1, 0, 0, 0, 5]);
    }

    #[wasm_bindgen_test]
    fn test_construct_message_header_large_length() {
        let header = construct_message_header(2, 1024 * 1024);

        assert_eq!(header[0], 2);
        let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
        assert_eq!(length, 1024 * 1024);
    }

    // ===== integration tests: full workflow =====

    #[wasm_bindgen_test]
    fn test_zero_copy_full_workflow_receive() {
        // Simulate the receive flow: JS -> Rust.
        let message = [1, 0, 0, 0, 5, 10, 20, 30, 40, 50]; // [header(5) | payload(5)]
        let js_array = js_sys::Uint8Array::from(&message[..]);

        // 1. Receive with minimal copying.
        let data = receive_zero_copy(&js_array);

        // 2. Parse the header.
        let (payload_type, length, offset) = parse_message_header(&data).unwrap();
        assert_eq!(payload_type, 1);
        assert_eq!(length, 5);
        assert_eq!(offset, 5);

        // 3. Extract the payload without another copy.
        let payload = extract_payload_zero_copy(data, offset);
        assert_eq!(payload.as_ref(), &[10, 20, 30, 40, 50]);
    }

    #[wasm_bindgen_test]
    fn test_zero_copy_full_workflow_send() {
        // Simulate the send flow: Rust -> JS.
        let payload = Bytes::from_static(&[10, 20, 30, 40, 50]);
        let header = construct_message_header(1, payload.len());

        // 1. Build the message.
        let msg = construct_message_zero_copy(&header, &payload);
        assert_eq!(msg.len(), 10);

        // 2. Send without another copy.
        let js_view = send_zero_copy(&msg);
        assert_eq!(js_view.length(), 10);
        assert_eq!(js_view.get_index(0), 1); // PayloadType
        assert_eq!(js_view.get_index(5), 10); // First payload byte.
    }

    // ===== Transferable Objects tests =====

    #[wasm_bindgen_test]
    fn test_send_with_transfer_basic() {
        let data = Bytes::from_static(&[1, 2, 3, 4, 5]);
        let (js_view, transfer_list) = send_with_transfer(&data);

        assert_eq!(js_view.length(), 5);
        assert_eq!(js_view.get_index(0), 1);
        assert_eq!(js_view.get_index(4), 5);

        // Verify the transferList contains the buffer.
        assert_eq!(transfer_list.length(), 1);
        assert!(transfer_list.get(0).is_truthy());
    }

    #[wasm_bindgen_test]
    fn test_send_with_transfer_large_data() {
        let large_data = vec![42u8; 100 * 1024]; // 100KB
        let data = Bytes::from(large_data);

        let (js_view, transfer_list) = send_with_transfer(&data);

        assert_eq!(js_view.length(), 100 * 1024);
        assert_eq!(js_view.get_index(0), 42);
        assert_eq!(js_view.get_index(100 * 1024 - 1), 42);

        assert_eq!(transfer_list.length(), 1);
    }

    #[wasm_bindgen_test]
    fn test_send_with_transfer_empty() {
        let data = Bytes::from_static(&[]);
        let (js_view, transfer_list) = send_with_transfer(&data);

        assert_eq!(js_view.length(), 0);
        assert_eq!(transfer_list.length(), 1);
    }

    #[wasm_bindgen_test]
    fn test_should_use_transfer_small_data() {
        // Small payloads should not use transfer.
        assert!(!should_use_transfer(1024)); // 1KB
        assert!(!should_use_transfer(5 * 1024)); // 5KB
        assert!(!should_use_transfer(9 * 1024)); // 9KB
    }

    #[wasm_bindgen_test]
    fn test_should_use_transfer_large_data() {
        // Large payloads should use transfer.
        assert!(should_use_transfer(10 * 1024)); // 10KB
        assert!(should_use_transfer(50 * 1024)); // 50KB
        assert!(should_use_transfer(1024 * 1024)); // 1MB
    }

    #[wasm_bindgen_test]
    fn test_should_use_transfer_edge_case() {
        // Boundary test.
        assert!(!should_use_transfer(10 * 1024 - 1)); // 10KB - 1
        assert!(should_use_transfer(10 * 1024)); // 10KB
        assert!(should_use_transfer(10 * 1024 + 1)); // 10KB + 1
    }
}
