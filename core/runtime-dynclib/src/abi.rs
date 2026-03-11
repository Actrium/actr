//! ABI error code definitions
//!
//! Kept in sync with `actr-hyper::executor::error_code`, used by
//! `entry_dynclib!` macro and `DynclibContext` internals.

/// ABI error codes
pub mod code {
    /// Operation succeeded
    pub const SUCCESS: i32 = 0;
    /// Generic unrecoverable error
    pub const GENERIC_ERROR: i32 = -1;
    /// Initialization failed
    pub const INIT_FAILED: i32 = -2;
    /// Message handling failed
    pub const HANDLE_FAILED: i32 = -3;
    /// Protocol / codec error
    pub const PROTOCOL_ERROR: i32 = -5;
}
