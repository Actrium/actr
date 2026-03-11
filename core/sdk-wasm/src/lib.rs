//! Actor WASM guest SDK
//!
//! For actors compiled to `wasm32-unknown-unknown`.
//! The crate hides linear-memory ABI details so implementers only need
//! to implement [`WasmActor`] and register it with the [`entry!`] macro.
//!
//! # Example
//!
//! ```rust,ignore
//! use actr_sdk_wasm::{WasmActor, ActorConfig, entry};
//!
//! struct EchoActor;
//!
//! impl WasmActor for EchoActor {
//!     fn init(_config: ActorConfig) -> Self {
//!         EchoActor
//!     }
//!
//!     fn handle(&mut self, request: &[u8]) -> Vec<u8> {
//!         request.to_vec() // echo
//!     }
//! }
//!
//! entry!(EchoActor);
//! ```

pub mod config;
pub use config::ActorConfig;

/// Trait implemented by WASM actors.
///
/// It matches the two host-side lifecycle entry points:
/// - `init`: called once when the actor starts and receives credential config
/// - `handle`: called for every message and returns the synchronous response
///
/// Both methods are **synchronous**. Async I/O is handled entirely by the
/// host runtime, while the guest performs pure computation only.
pub trait WasmActor: Sized + 'static {
    /// Initialize the actor and return its instance.
    ///
    /// The host guarantees this is called once before any `handle` invocation.
    fn init(config: ActorConfig) -> Self;

    /// Handle one request and return the response bytes.
    ///
    /// - `request`: raw request bytes written by the host, usually protobuf
    /// - return value: response bytes; an empty `Vec` means fire-and-forget
    fn handle(&mut self, request: &[u8]) -> Vec<u8>;
}

/// Register an actor type and generate the required ABI exports.
///
/// The macro expands to:
/// - `actr_init(config_ptr, config_len) -> i32`
/// - `actr_handle(req_ptr, req_len, resp_ptr_out, resp_len_out) -> i32`
/// - `actr_alloc(size) -> i32`
/// - `actr_free(ptr, size)`
///
/// # Example
///
/// ```rust,ignore
/// entry!(MyActor);
/// ```
#[macro_export]
macro_rules! entry {
    ($actor:ty) => {
        // ABI status codes must stay aligned with the host-side abi::code values.
        const __SUCCESS: i32 = 0;
        const __GENERIC_ERROR: i32 = -1;
        const __INIT_FAILED: i32 = -2;
        const __HANDLE_FAILED: i32 = -3;
        const __PROTOCOL_ERROR: i32 = -5;

        static mut __ACTR_INSTANCE: Option<$actor> = None;

        /// `actr_init`: called once by the host with JSON-encoded `ActorConfig`.
        #[no_mangle]
        pub unsafe extern "C" fn actr_init(config_ptr: i32, config_len: i32) -> i32 {
            let bytes = core::slice::from_raw_parts(
                config_ptr as *const u8,
                config_len as usize,
            );
            let config = match serde_json::from_slice::<$crate::ActorConfig>(bytes) {
                Ok(c) => c,
                Err(_) => return __PROTOCOL_ERROR,
            };
            __ACTR_INSTANCE = Some(<$actor as $crate::WasmActor>::init(config));
            __SUCCESS
        }

        /// `actr_handle`: called by the host for every dispatched message.
        #[no_mangle]
        pub unsafe extern "C" fn actr_handle(
            req_ptr: i32,
            req_len: i32,
            resp_ptr_out: i32,
            resp_len_out: i32,
        ) -> i32 {
            let actor = match __ACTR_INSTANCE.as_mut() {
                Some(a) => a,
                None => return __GENERIC_ERROR,
            };
            let req =
                core::slice::from_raw_parts(req_ptr as *const u8, req_len as usize);

            let resp = <$actor as $crate::WasmActor>::handle(actor, req);
            let resp_len = resp.len();

            if resp_len > 0 {
                // Copy the response into a dedicated allocation that the host
                // reads and later frees through `actr_free`.
                let layout =
                    core::alloc::Layout::from_size_align(resp_len, 1).unwrap();
                let out_ptr = std::alloc::alloc(layout);
                if out_ptr.is_null() {
                    return __HANDLE_FAILED;
                }
                out_ptr.copy_from_nonoverlapping(resp.as_ptr(), resp_len);

                *(resp_ptr_out as *mut i32) = out_ptr as i32;
                *(resp_len_out as *mut i32) = resp_len as i32;
            } else {
                *(resp_ptr_out as *mut i32) = 0;
                *(resp_len_out as *mut i32) = 0;
            }

            __SUCCESS
        }

        /// `actr_alloc`: lets the host allocate a buffer in WASM linear memory.
        #[no_mangle]
        pub unsafe extern "C" fn actr_alloc(size: i32) -> i32 {
            if size <= 0 {
                return 0;
            }
            let layout =
                core::alloc::Layout::from_size_align(size as usize, 1).unwrap();
            std::alloc::alloc(layout) as i32
        }

        /// `actr_free`: lets the host free a previously allocated buffer.
        #[no_mangle]
        pub unsafe extern "C" fn actr_free(ptr: i32, size: i32) {
            if ptr != 0 && size > 0 {
                let layout =
                    core::alloc::Layout::from_size_align(size as usize, 1).unwrap();
                std::alloc::dealloc(ptr as *mut u8, layout);
            }
        }
    };
}
