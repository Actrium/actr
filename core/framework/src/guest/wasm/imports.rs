//! WASM host import declarations.
//!
//! The runtime ABI exposes a single host invocation entry for business operations.

unsafe extern "C" {
    /// Invoke a host operation encoded as [`crate::guest::abi::AbiFrame`].
    ///
    /// Arguments:
    /// - `frame_ptr/frame_len`: encoded runtime frame in guest linear memory
    /// - `reply_buf_ptr/reply_buf_cap`: guest-allocated reply buffer
    /// - `reply_len_out`: host writes the actual encoded reply length here
    ///
    /// Returns:
    /// - `0` on transport-level success
    /// - negative ABI error code on transport-level failure
    pub fn actr_host_invoke(
        frame_ptr: i32,
        frame_len: i32,
        reply_buf_ptr: i32,
        reply_buf_cap: i32,
        reply_len_out: i32,
    ) -> i32;
}
