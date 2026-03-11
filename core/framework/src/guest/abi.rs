//! Guest-Host ABI error code definitions
//!
//! Shared by both WASM and dynclib guest runtimes.
//! Kept in sync with host-side error codes in `actr-hyper`.

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
    /// Memory allocation failed (WASM only)
    pub const ALLOC_FAILED: i32 = -4;
    /// Protocol / codec error
    pub const PROTOCOL_ERROR: i32 = -5;
}
