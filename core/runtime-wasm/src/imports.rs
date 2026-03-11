//! Host import declarations
//!
//! Host functions declared via `extern "C"` in the WASM module (implemented by `actr-runtime`).
//!
//! # Memory conventions
//!
//! - All pointers refer to addresses in WASM linear memory (i32 offsets)
//! - Host allocates input buffers via `actr_alloc`, guest frees them via `actr_free`
//! - Guest passes output buffer pointer and max length via parameters, host writes directly
//!
//! # Asyncify protocol
//!
//! `actr_host_call` / `actr_host_call_raw` support transparent asyncify suspend/resume:
//! - **Normal mode**: Host calls `asyncify_start_unwind` within the import callback, WASM saves call stack then returns
//! - **Rewinding mode**: Host re-enters WASM, calls `asyncify_stop_rewind`, import returns actual result directly
//! - From the guest side, import calls appear identical to normal synchronous function calls

unsafe extern "C" {
    /// Get the current Actor's ActrId (protobuf encoded)
    ///
    /// Host writes ActrId bytes into `[out_ptr, out_ptr + out_max)`
    /// Returns actual bytes written, 0 indicates error
    pub fn actr_host_self_id(out_ptr: i32, out_max: i32) -> i32;

    /// Get the caller Actor's ActrId (protobuf encoded)
    ///
    /// Host writes caller ActrId bytes into the output buffer
    /// Returns actual bytes written, -1 indicates no caller (system call, e.g., lifecycle hooks)
    pub fn actr_host_caller_id(out_ptr: i32, out_max: i32) -> i32;

    /// Get the current request ID (UTF-8 string)
    ///
    /// Host writes request ID into the output buffer
    /// Returns actual bytes written
    pub fn actr_host_request_id(out_ptr: i32, out_max: i32) -> i32;

    /// Send RPC request and wait for response (asyncify transparent suspend/resume)
    ///
    /// Arguments:
    /// - `route_key_ptr/len`: route key (UTF-8)
    /// - `dest_ptr/len`: target Dest encoding (see [`crate::context::encode_dest`])
    /// - `payload_ptr/len`: protobuf encoded request payload
    /// - `out_ptr/out_max`: response output buffer (guest pre-allocated)
    /// - `out_len_ptr`: address where host writes actual response length (i32*)
    ///
    /// Returns: 0 on success, < 0 see [`crate::abi::code`]
    pub fn actr_host_call(
        route_key_ptr: i32,
        route_key_len: i32,
        dest_ptr: i32,
        dest_len: i32,
        payload_ptr: i32,
        payload_len: i32,
        out_ptr: i32,
        out_max: i32,
        out_len_ptr: i32,
    ) -> i32;

    /// Send one-way message (fire-and-forget, no response)
    ///
    /// Returns: 0 on success, < 0 see [`crate::abi::code`]
    pub fn actr_host_tell(
        route_key_ptr: i32,
        route_key_len: i32,
        dest_ptr: i32,
        dest_len: i32,
        payload_ptr: i32,
        payload_len: i32,
    ) -> i32;

    /// Raw RPC call (routed by ActrId, bypasses Dest resolution)
    ///
    /// Returns: 0 on success, < 0 see [`crate::abi::code`]
    pub fn actr_host_call_raw(
        route_key_ptr: i32,
        route_key_len: i32,
        target_ptr: i32,
        target_len: i32,
        payload_ptr: i32,
        payload_len: i32,
        out_ptr: i32,
        out_max: i32,
        out_len_ptr: i32,
    ) -> i32;

    /// Discover route candidates by Actor type (Signaling service discovery)
    ///
    /// - `type_ptr/len`: ActrType protobuf encoded
    /// - `out_ptr/out_max`: output buffer for ActrId protobuf encoding
    ///
    /// Returns actual bytes written, < 0 error code
    pub fn actr_host_discover(type_ptr: i32, type_len: i32, out_ptr: i32, out_max: i32) -> i32;
}
