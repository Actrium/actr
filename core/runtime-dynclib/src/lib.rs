//! # actr-runtime-dynclib
//!
//! Actor-RTC cdylib guest-side runtime, runs in native shared libraries (.so/.dylib/.dll).
//!
//! This crate is the native counterpart of `actr-runtime-wasm`:
//! - **`actr-runtime-wasm`**: WASM guest-side, communicates with host via host imports
//! - **`actr-runtime-dynclib`**: native cdylib guest-side, communicates with host via HostVTable function pointers
//!
//! ## Architecture position
//!
//! ```text
//! actor business code (actr-framework interface)
//!         | compiled to cdylib (.so/.dylib/.dll)
//! actr-runtime-dynclib (this crate, compiled into shared library)
//!         | HostVTable function pointers
//! actr-hyper (host-side, dlopen loads and calls exported functions)
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use actr_runtime_dynclib::entry_dynclib;
//!
//! // 1. Implement Handler (via actr-framework interface)
//! struct MyService;
//! // impl EchoServiceHandler for MyService { ... }
//!
//! // 2. Register Workload, generate cdylib ABI exports
//! entry_dynclib!(EchoServiceWorkload<MyService>);
//! ```
//!
//! ## Differences from WASM guest
//!
//! - **Shared address space**: SO shares the process address space with host, no alloc/free intermediary needed
//! - **VTable callbacks**: Uses function pointer table instead of WASM host imports
//! - **Response allocated by guest**: `actr_handle` allocates response on guest heap, host frees via `actr_free_response`

pub mod abi;
pub mod context;
pub mod vtable;

// Convenience re-exports
pub use context::DynclibContext;
pub use vtable::HostVTable;

/// Generate cdylib ABI export functions for a Workload type
///
/// # Arguments
///
/// - `$workload_type`: Type implementing `actr_framework::Workload + Send + Sync + 'static`
/// - `$init_expr` (optional): Expression to construct the Workload instance; uses `Default::default()` if omitted
///
/// # Generated exports
///
/// - `actr_init(vtable, config_ptr, config_len) -> i32`
/// - `actr_handle(req_ptr, req_len, resp_out, resp_len_out) -> i32`
/// - `actr_free_response(ptr, len)`
#[macro_export]
macro_rules! entry_dynclib {
    // Single-argument form: use Default::default() for initialization
    ($workload_type:ty) => {
        $crate::entry_dynclib!($workload_type, <$workload_type as Default>::default());
    };

    // Two-argument form: use custom initialization expression
    ($workload_type:ty, $init_expr:expr) => {
        static mut __ACTR_WORKLOAD: Option<$workload_type> = None;
        static mut __ACTR_VTABLE: Option<*const $crate::vtable::HostVTable> = None;

        /// Initialize actor
        ///
        /// Host calls this after dlopen, passing HostVTable and optional config data.
        /// Returns 0 on success, negative on error.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn actr_init(
            vtable: *const $crate::vtable::HostVTable,
            _config_ptr: *const u8,
            _config_len: usize,
        ) -> i32 {
            if vtable.is_null() {
                return $crate::abi::code::INIT_FAILED;
            }

            let workload: $workload_type = $init_expr;
            unsafe {
                __ACTR_VTABLE = Some(vtable);
                __ACTR_WORKLOAD = Some(workload);
            }
            $crate::abi::code::SUCCESS
        }

        /// Handle one RPC request
        ///
        /// - `req_ptr/req_len`: protobuf-encoded `RpcEnvelope`
        /// - `resp_out`: pointer to `*mut u8`, function writes response buffer address here
        /// - `resp_len_out`: pointer to `usize`, function writes response data length here
        ///
        /// Response buffer is allocated on the guest heap; host must call `actr_free_response` after use.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn actr_handle(
            req_ptr: *const u8,
            req_len: usize,
            resp_out: *mut *mut u8,
            resp_len_out: *mut usize,
        ) -> i32 {
            use actr_framework::{MessageDispatcher, Workload};
            use actr_protocol::prost::Message as ProstMessage;

            // Get vtable
            let vtable = match unsafe { __ACTR_VTABLE } {
                Some(vt) => vt,
                None => return $crate::abi::code::INIT_FAILED,
            };

            // Read request envelope
            if req_ptr.is_null() {
                return $crate::abi::code::PROTOCOL_ERROR;
            }
            let req_bytes = unsafe { std::slice::from_raw_parts(req_ptr, req_len) };

            let envelope = match actr_protocol::RpcEnvelope::decode(req_bytes) {
                Ok(e) => e,
                Err(_) => return $crate::abi::code::PROTOCOL_ERROR,
            };

            // Build DynclibContext (obtain current call context data from vtable)
            let ctx = match unsafe { $crate::context::DynclibContext::from_vtable(vtable) } {
                Ok(c) => c,
                Err(_) => return $crate::abi::code::HANDLE_FAILED,
            };

            // Get workload reference
            let workload = unsafe {
                match __ACTR_WORKLOAD.as_ref() {
                    Some(w) => w,
                    None => return $crate::abi::code::INIT_FAILED,
                }
            };

            // Route and execute via MessageDispatcher
            type Dispatcher = <$workload_type as Workload>::Dispatcher;

            // cdylib is native environment, can use tokio or synchronous execution
            // Here we use the same single-threaded poll strategy as WASM:
            // All host callbacks (vtable function pointers) are synchronous, Future completes in one poll.
            let resp_result = {
                let fut = Dispatcher::dispatch(workload, envelope, &ctx);
                // Construct noop waker to synchronously drive the future
                let waker = {
                    use std::task::{RawWaker, RawWakerVTable, Waker};
                    const VTABLE: RawWakerVTable =
                        RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
                    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
                };
                let mut cx = std::task::Context::from_waker(&waker);
                let mut pinned = std::pin::pin!(fut);
                match pinned.as_mut().poll(&mut cx) {
                    std::task::Poll::Ready(v) => v,
                    std::task::Poll::Pending => {
                        return $crate::abi::code::HANDLE_FAILED;
                    }
                }
            };

            let resp_bytes = match resp_result {
                Ok(b) => b,
                Err(_) => return $crate::abi::code::HANDLE_FAILED,
            };

            // Allocate response buffer on guest heap
            let resp_len = resp_bytes.len();
            let layout = match std::alloc::Layout::from_size_align(resp_len.max(1), 1) {
                Ok(l) => l,
                Err(_) => return $crate::abi::code::GENERIC_ERROR,
            };
            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return $crate::abi::code::GENERIC_ERROR;
            }

            unsafe {
                std::ptr::copy_nonoverlapping(resp_bytes.as_ptr(), ptr, resp_len);
                *resp_out = ptr;
                *resp_len_out = resp_len;
            }

            $crate::abi::code::SUCCESS
        }

        /// Free guest-allocated response buffer
        ///
        /// Host calls this after using the response data returned by `actr_handle`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn actr_free_response(ptr: *mut u8, len: usize) {
            if ptr.is_null() || len == 0 {
                return;
            }
            let layout = match std::alloc::Layout::from_size_align(len, 1) {
                Ok(l) => l,
                Err(_) => return,
            };
            unsafe { std::alloc::dealloc(ptr, layout) };
        }
    };
}
