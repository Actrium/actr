//! HostVTable -- Host callback function pointer table
//!
//! The host (Hyper) passes this table at `actr_init` time; the guest caches it in
//! a thread-local, and `DynclibContext` uses the function pointers for RPC, discovery, etc.
//!
//! Unlike WASM host imports, dynclib shares the address space with the host,
//! so no alloc/free intermediary is needed -- pointers are passed directly.

/// Host callback function pointer table
///
/// # Safety
///
/// - All function pointers remain valid from `actr_init` until process exit
/// - The host guarantees thread safety: the same actor instance is never entered concurrently
/// - A single vtable corresponds to one logical actor instance; hosts that need multiple
///   actors must create multiple guest instances and pass each one its own `HostVTable`
/// - `*mut *mut u8` output pointers refer to host-allocated memory; the guest must free them via `free_host_buf`
#[repr(C)]
pub struct HostVTable {
    /// RPC call and wait for response
    ///
    /// `call(route_key_ptr, route_key_len, dest_ptr, dest_len,
    ///       payload_ptr, payload_len, resp_ptr_out, resp_len_out) -> error_code`
    ///
    /// Host writes response buffer address and length at `resp_ptr_out` / `resp_len_out`.
    /// Guest must call `free_host_buf` after use.
    pub call: unsafe extern "C" fn(
        *const u8,
        usize,
        *const u8,
        usize,
        *const u8,
        usize,
        *mut *mut u8,
        *mut usize,
    ) -> i32,

    /// One-way message (fire-and-forget)
    ///
    /// `tell(route_key_ptr, route_key_len, dest_ptr, dest_len,
    ///       payload_ptr, payload_len) -> error_code`
    pub tell: unsafe extern "C" fn(*const u8, usize, *const u8, usize, *const u8, usize) -> i32,

    /// Service discovery
    ///
    /// `discover(type_ptr, type_len, resp_ptr_out, resp_len_out) -> error_code`
    ///
    /// Host writes protobuf-encoded ActrId at `resp_ptr_out` / `resp_len_out`.
    /// Guest must call `free_host_buf` after use.
    pub discover: unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32,

    /// Get current actor's ActrId
    ///
    /// `self_id(buf_ptr_out, buf_len_out) -> error_code`
    ///
    /// Host writes protobuf-encoded ActrId; guest frees via `free_host_buf`.
    pub self_id: unsafe extern "C" fn(*mut *mut u8, *mut usize) -> i32,

    /// Get caller's ActrId
    ///
    /// `caller_id(buf_ptr_out, buf_len_out) -> i32`
    ///
    /// Returns 0 when caller exists, 1 when no caller (internal system call).
    /// When caller exists, host writes protobuf-encoded ActrId; guest frees via `free_host_buf`.
    pub caller_id: unsafe extern "C" fn(*mut *mut u8, *mut usize) -> i32,

    /// Get current request ID
    ///
    /// `request_id(buf_ptr_out, buf_len_out) -> error_code`
    ///
    /// Host writes UTF-8 encoded request ID; guest frees via `free_host_buf`.
    pub request_id: unsafe extern "C" fn(*mut *mut u8, *mut usize) -> i32,

    /// Free host-allocated buffer
    ///
    /// `free_host_buf(ptr, len)`
    ///
    /// All buffers returned by the host via `*_out` pointers must be freed by the guest using this function.
    pub free_host_buf: unsafe extern "C" fn(*mut u8, usize),
}
