//! Guest-side runtime module
//!
//! Provides the unified `entry!` macro and platform-specific Context implementations.
//! Actor developers write ONE `entry!(MyActor)` -- platform ABI is auto-selected by cfg.
//!
//! # Supported platforms
//!
//! - **WASM** (`target_arch = "wasm32"`): host imports + asyncify
//! - **cdylib** (`feature = "cdylib"`): HostVTable function pointers

pub mod abi;
pub mod vtable;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

#[cfg(feature = "cdylib")]
pub mod dynclib;

/// Generate ABI export functions for a Workload type
///
/// Platform ABI is auto-selected:
/// - `#[cfg(target_arch = "wasm32")]`: WASM ABI exports (actr_alloc, actr_free, actr_init, actr_handle)
/// - `#[cfg(feature = "cdylib")]`: dynclib ABI exports (actr_init with vtable, actr_handle, actr_free_response)
///
/// # Arguments
///
/// - `$workload_type`: Type implementing `actr_framework::Workload + Send + Sync + 'static`
/// - `$init_expr` (optional): Expression to construct the Workload instance; uses `Default::default()` if omitted
///
/// # Usage
///
/// ```rust,ignore
/// use actr_framework::entry;
///
/// // Use Default initialization (requires MyWorkload: Default)
/// entry!(EchoServiceWorkload<MyService>);
///
/// // Or provide a custom initialization expression
/// entry!(EchoServiceWorkload<MyService>, EchoServiceWorkload(MyService::new()));
/// ```
#[macro_export]
macro_rules! entry {
    // Single-argument form: use Default::default() for initialization
    ($workload_type:ty) => {
        $crate::entry!($workload_type, <$workload_type as Default>::default());
    };

    // Two-argument form: use custom initialization expression
    ($workload_type:ty, $init_expr:expr) => {
        // ── WASM ABI exports ──────────────────────────────────────────────
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            static mut __ACTR_WORKLOAD: Option<$workload_type> = None;

            /// Allocate WASM linear memory (host calls before writing data)
            #[unsafe(no_mangle)]
            pub extern "C" fn actr_alloc(size: i32) -> i32 {
                let layout =
                    std::alloc::Layout::from_size_align(size as usize, 1).expect("invalid layout");
                let ptr = unsafe { std::alloc::alloc(layout) };
                if ptr.is_null() {
                    $crate::guest::abi::code::ALLOC_FAILED
                } else {
                    ptr as i32
                }
            }

            /// Free WASM linear memory (host calls after read/write is done)
            #[unsafe(no_mangle)]
            pub extern "C" fn actr_free(ptr: i32, size: i32) {
                if ptr == 0 || size <= 0 {
                    return;
                }
                let layout =
                    std::alloc::Layout::from_size_align(size as usize, 1).expect("invalid layout");
                unsafe { std::alloc::dealloc(ptr as *mut u8, layout) };
            }

            /// Initialize actor (host calls before first actr_handle call)
            ///
            /// `config_ptr/len`: JSON-encoded WasmActorConfig (reserved for future extension)
            #[unsafe(no_mangle)]
            pub extern "C" fn actr_init(_config_ptr: i32, _config_len: i32) -> i32 {
                let workload: $workload_type = $init_expr;
                unsafe {
                    __ACTR_WORKLOAD = Some(workload);
                }
                $crate::guest::abi::code::SUCCESS
            }

            /// Handle one RPC request
            ///
            /// - `req_ptr/len`: protobuf-encoded `RpcEnvelope`
            /// - `resp_ptr_out`: host-provided `i32*`, WASM writes response buffer address here
            /// - `resp_len_out`: host-provided `i32*`, WASM writes response data length here
            #[unsafe(no_mangle)]
            pub extern "C" fn actr_handle(
                req_ptr: i32,
                req_len: i32,
                resp_ptr_out: i32,
                resp_len_out: i32,
            ) -> i32 {
                use actr_protocol::prost::Message as ProstMessage;
                use $crate::{MessageDispatcher, Workload};

                // Read request envelope
                let req_bytes: &[u8] =
                    unsafe { std::slice::from_raw_parts(req_ptr as *const u8, req_len as usize) };

                let envelope = match actr_protocol::RpcEnvelope::decode(req_bytes) {
                    Ok(e) => e,
                    Err(_) => return $crate::guest::abi::code::PROTOCOL_ERROR,
                };

                // Build WasmContext (obtain current call context data from host)
                let ctx = match $crate::guest::wasm::context::WasmContext::from_host() {
                    Ok(c) => c,
                    Err(_) => return $crate::guest::abi::code::HANDLE_FAILED,
                };

                // Get workload reference
                let workload = unsafe {
                    match __ACTR_WORKLOAD.as_ref() {
                        Some(w) => w,
                        None => return $crate::guest::abi::code::INIT_FAILED,
                    }
                };

                // Route and execute via MessageDispatcher
                type Dispatcher = <$workload_type as Workload>::Dispatcher;
                let resp_result = $crate::guest::wasm::executor::block_on(Dispatcher::dispatch(
                    workload, envelope, &ctx,
                ));

                let resp_bytes = match resp_result {
                    Ok(b) => b,
                    Err(_) => return $crate::guest::abi::code::HANDLE_FAILED,
                };

                // Allocate response buffer in WASM linear memory, return to host
                let resp_len = resp_bytes.len();
                let layout = std::alloc::Layout::from_size_align(resp_len.max(1), 1)
                    .expect("invalid layout");
                let resp_ptr = unsafe { std::alloc::alloc(layout) };
                if resp_ptr.is_null() {
                    return $crate::guest::abi::code::ALLOC_FAILED;
                }

                // Write response data to WASM linear memory
                unsafe {
                    std::ptr::copy_nonoverlapping(resp_bytes.as_ptr(), resp_ptr, resp_len);
                    // Write response buffer address and length to host-provided output pointers
                    *(resp_ptr_out as *mut i32) = resp_ptr as i32;
                    *(resp_len_out as *mut i32) = resp_len as i32;
                }

                $crate::guest::abi::code::SUCCESS
            }
        };

        // ── cdylib ABI exports ────────────────────────────────────────────
        #[cfg(feature = "cdylib")]
        const _: () = {
            static mut __ACTR_WORKLOAD: Option<$workload_type> = None;
            static mut __ACTR_VTABLE: Option<*const $crate::guest::vtable::HostVTable> = None;

            /// Initialize actor
            ///
            /// Host calls this after dlopen, passing HostVTable and optional config data.
            /// Returns 0 on success, negative on error.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn actr_init(
                vtable: *const $crate::guest::vtable::HostVTable,
                _config_ptr: *const u8,
                _config_len: usize,
            ) -> i32 {
                if vtable.is_null() {
                    return $crate::guest::abi::code::INIT_FAILED;
                }

                let workload: $workload_type = $init_expr;
                unsafe {
                    __ACTR_VTABLE = Some(vtable);
                    __ACTR_WORKLOAD = Some(workload);
                }
                $crate::guest::abi::code::SUCCESS
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
                use actr_protocol::prost::Message as ProstMessage;
                use $crate::{MessageDispatcher, Workload};

                // Get vtable
                let vtable = match unsafe { __ACTR_VTABLE } {
                    Some(vt) => vt,
                    None => return $crate::guest::abi::code::INIT_FAILED,
                };

                // Read request envelope
                if req_ptr.is_null() {
                    return $crate::guest::abi::code::PROTOCOL_ERROR;
                }
                let req_bytes = unsafe { std::slice::from_raw_parts(req_ptr, req_len) };

                let envelope = match actr_protocol::RpcEnvelope::decode(req_bytes) {
                    Ok(e) => e,
                    Err(_) => return $crate::guest::abi::code::PROTOCOL_ERROR,
                };

                // Build DynclibContext (obtain current call context data from vtable)
                let ctx = match unsafe {
                    $crate::guest::dynclib::context::DynclibContext::from_vtable(vtable)
                } {
                    Ok(c) => c,
                    Err(_) => return $crate::guest::abi::code::HANDLE_FAILED,
                };

                // Get workload reference
                let workload = unsafe {
                    match __ACTR_WORKLOAD.as_ref() {
                        Some(w) => w,
                        None => return $crate::guest::abi::code::INIT_FAILED,
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
                        const VTABLE: RawWakerVTable = RawWakerVTable::new(
                            |p| RawWaker::new(p, &VTABLE),
                            |_| {},
                            |_| {},
                            |_| {},
                        );
                        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
                    };
                    let mut cx = std::task::Context::from_waker(&waker);
                    let mut pinned = std::pin::pin!(fut);
                    match pinned.as_mut().poll(&mut cx) {
                        std::task::Poll::Ready(v) => v,
                        std::task::Poll::Pending => {
                            return $crate::guest::abi::code::HANDLE_FAILED;
                        }
                    }
                };

                let resp_bytes = match resp_result {
                    Ok(b) => b,
                    Err(_) => return $crate::guest::abi::code::HANDLE_FAILED,
                };

                // Allocate response buffer on guest heap
                let resp_len = resp_bytes.len();
                let layout = match std::alloc::Layout::from_size_align(resp_len.max(1), 1) {
                    Ok(l) => l,
                    Err(_) => return $crate::guest::abi::code::GENERIC_ERROR,
                };
                let ptr = unsafe { std::alloc::alloc(layout) };
                if ptr.is_null() {
                    return $crate::guest::abi::code::GENERIC_ERROR;
                }

                unsafe {
                    std::ptr::copy_nonoverlapping(resp_bytes.as_ptr(), ptr, resp_len);
                    *resp_out = ptr;
                    *resp_len_out = resp_len;
                }

                $crate::guest::abi::code::SUCCESS
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
    };
}
