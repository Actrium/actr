//! Guest-side runtime module.
//!
//! Provides the unified [`entry!`] macro and platform-specific runtime
//! glue. Actor developers write one `entry!(MyActor)`; the macro selects
//! the correct ABI at compile time based on the target.
//!
//! # Execution contract
//!
//! - One loaded guest instance corresponds to one logical actor instance.
//! - The runtime serialises dispatch into the guest instance. Concurrent
//!   dispatches within the same instance are forbidden by the host
//!   (wasmtime enforces this via `&mut Store<HostState>`; dynclib hosts
//!   enforce via handle ownership).
//!
//! # Supported platforms
//!
//! - **WASM Component Model** (`target_arch = "wasm32"`): wit-bindgen
//!   generates the `Guest` trait + `host` imports from
//!   `core/framework/wit/actr-workload.wit`; the [`entry!`] macro
//!   produces an adapter that bridges the user's [`Workload`] impl into
//!   the generated `Guest`. Targets `wasm32-wasip2` and requires
//!   `wasm-component-ld 0.5.22+` as the linker (see
//!   `experiments/component-spike-async/REPORT.md`).
//! - **cdylib** (`feature = "cdylib"`): HostVTable function-pointer
//!   bridge used for native shared-library guests (iOS / Android).

pub mod abi;
pub mod vtable;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

#[cfg(feature = "cdylib")]
pub mod dynclib;

// Re-exports used by the `entry!` macro so macro expansions under a
// user crate can reference adapter / binding items via a stable path
// without having to know the internal module layout.
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub mod __wasm_macro_support {
    // The `entry!` macro expands inside the user's crate, so every name
    // it references must be reachable from an external crate path. Both
    // the adapter helpers and the generated WIT types are re-exported
    // here so the macro has a single stable `$crate::guest::__wasm_macro_support::*`
    // prefix to poke at.
    pub use super::wasm::adapter::{
        run_dispatch, run_on_credential_expiring, run_on_credential_renewed, run_on_error,
        run_on_mailbox_backpressure, run_on_ready, run_on_signaling_connected,
        run_on_signaling_connecting, run_on_signaling_disconnected, run_on_start,
        run_on_stop, run_on_webrtc_connected, run_on_webrtc_connecting,
        run_on_webrtc_disconnected, run_on_websocket_connected,
        run_on_websocket_connecting, run_on_websocket_disconnected, WorkloadCell,
    };
    pub use super::wasm::generated::exports::actr::workload::workload::Guest;
    pub use super::wasm::generated::actr::workload::types::{
        ActrError as WitActrError, BackpressureEvent as WitBackpressureEvent,
        CredentialEvent as WitCredentialEvent, ErrorEvent as WitErrorEvent,
        PeerEvent as WitPeerEvent, RpcEnvelope as WitRpcEnvelope,
    };
}

/// Generate Component Model exports for a [`Workload`][crate::Workload]
/// type.
///
/// Platform ABI is auto-selected by target:
///
/// - `#[cfg(target_arch = "wasm32")]` — expands to an
///   `impl Guest for __ActrEntryAdapter { ... }` bridging the user's
///   [`Workload`][crate::Workload] into the `actr:workload/workload`
///   export contract. The runtime calls `Dispatcher::dispatch` through
///   the `dispatch` export and every observation hook through its
///   matching WIT export.
/// - `#[cfg(feature = "cdylib")]` — expands to the legacy
///   `actr_init` / `actr_handle` / `actr_free_response` C-ABI exports
///   used by native shared-library hosts.
///
/// # Arguments
///
/// - `$workload_type`: type implementing
///   `actr_framework::Workload + Send + Sync + 'static`.
/// - `$init_expr` (optional): expression returning a fresh instance of
///   `$workload_type`. Defaults to `<$workload_type as Default>::default()`.
///
/// # Usage
///
/// ```rust,ignore
/// use actr_framework::entry;
///
/// entry!(EchoServiceWorkload<MyService>);
///
/// // Or with a custom constructor:
/// entry!(
///     EchoServiceWorkload<MyService>,
///     EchoServiceWorkload::new(MyService::new())
/// );
/// ```
#[macro_export]
macro_rules! entry {
    // Single-argument form: default-construct the workload.
    ($workload_type:ty) => {
        $crate::entry!($workload_type, <$workload_type as ::core::default::Default>::default());
    };

    // Two-argument form: caller supplies the init expression.
    ($workload_type:ty, $init_expr:expr) => {
        // ── WASM Component Model exports ──────────────────────────────────
        //
        // wit-bindgen generates an `exports::actr::workload::workload::Guest`
        // trait with 17 async methods (one `dispatch` + sixteen hooks). We
        // emit a single zero-sized adapter struct in user-crate scope and
        // route every method through helpers in `actr_framework::guest::wasm::adapter`.
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            // Module-local singleton cell. Lazy-init on first call; subsequent
            // dispatches reuse the same instance.
            static __ACTR_WORKLOAD: $crate::guest::__wasm_macro_support::WorkloadCell<$workload_type> =
                $crate::guest::__wasm_macro_support::WorkloadCell::new();

            fn __actr_workload() -> &'static $workload_type {
                __ACTR_WORKLOAD.get_or_init(|| -> $workload_type { $init_expr })
            }

            struct __ActrEntryAdapter;

            impl $crate::guest::__wasm_macro_support::Guest for __ActrEntryAdapter {
                async fn dispatch(
                    envelope: $crate::guest::__wasm_macro_support::WitRpcEnvelope,
                ) -> ::core::result::Result<
                    ::std::vec::Vec<u8>,
                    $crate::guest::__wasm_macro_support::WitActrError,
                > {
                    $crate::guest::__wasm_macro_support::run_dispatch(
                        __actr_workload(),
                        envelope,
                    )
                    .await
                }

                async fn on_start() -> ::core::result::Result<
                    (),
                    $crate::guest::__wasm_macro_support::WitActrError,
                > {
                    $crate::guest::__wasm_macro_support::run_on_start(__actr_workload()).await
                }

                async fn on_ready() -> ::core::result::Result<
                    (),
                    $crate::guest::__wasm_macro_support::WitActrError,
                > {
                    $crate::guest::__wasm_macro_support::run_on_ready(__actr_workload()).await
                }

                async fn on_stop() -> ::core::result::Result<
                    (),
                    $crate::guest::__wasm_macro_support::WitActrError,
                > {
                    $crate::guest::__wasm_macro_support::run_on_stop(__actr_workload()).await
                }

                async fn on_error(
                    event: $crate::guest::__wasm_macro_support::WitErrorEvent,
                ) -> ::core::result::Result<
                    (),
                    $crate::guest::__wasm_macro_support::WitActrError,
                > {
                    $crate::guest::__wasm_macro_support::run_on_error(__actr_workload(), event)
                        .await
                }

                async fn on_signaling_connecting() {
                    $crate::guest::__wasm_macro_support::run_on_signaling_connecting(
                        __actr_workload(),
                    )
                    .await
                }

                async fn on_signaling_connected() {
                    $crate::guest::__wasm_macro_support::run_on_signaling_connected(
                        __actr_workload(),
                    )
                    .await
                }

                async fn on_signaling_disconnected() {
                    $crate::guest::__wasm_macro_support::run_on_signaling_disconnected(
                        __actr_workload(),
                    )
                    .await
                }

                async fn on_websocket_connecting(
                    event: $crate::guest::__wasm_macro_support::WitPeerEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_websocket_connecting(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_websocket_connected(
                    event: $crate::guest::__wasm_macro_support::WitPeerEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_websocket_connected(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_websocket_disconnected(
                    event: $crate::guest::__wasm_macro_support::WitPeerEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_websocket_disconnected(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_webrtc_connecting(
                    event: $crate::guest::__wasm_macro_support::WitPeerEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_webrtc_connecting(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_webrtc_connected(
                    event: $crate::guest::__wasm_macro_support::WitPeerEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_webrtc_connected(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_webrtc_disconnected(
                    event: $crate::guest::__wasm_macro_support::WitPeerEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_webrtc_disconnected(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_credential_renewed(
                    event: $crate::guest::__wasm_macro_support::WitCredentialEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_credential_renewed(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_credential_expiring(
                    event: $crate::guest::__wasm_macro_support::WitCredentialEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_credential_expiring(
                        __actr_workload(),
                        event,
                    )
                    .await
                }

                async fn on_mailbox_backpressure(
                    event: $crate::guest::__wasm_macro_support::WitBackpressureEvent,
                ) {
                    $crate::guest::__wasm_macro_support::run_on_mailbox_backpressure(
                        __actr_workload(),
                        event,
                    )
                    .await
                }
            }

            $crate::guest::wasm::generated::export!(__ActrEntryAdapter with_types_in $crate::guest::wasm::generated);
        };

        // ── cdylib ABI exports ────────────────────────────────────────────
        //
        // Unchanged from pre-Phase-1; the Component Model rewrite is WASM-
        // only and does not touch the native shared-library path.
        #[cfg(feature = "cdylib")]
        const _: () = {
            static mut __ACTR_WORKLOAD: Option<$workload_type> = None;
            static mut __ACTR_VTABLE: Option<*const $crate::guest::vtable::HostVTable> = None;

            /// Initialize actor
            ///
            /// Host calls this after dlopen, passing HostVTable and init payload.
            /// Returns 0 on success, negative on error.
            /// Repeated calls return `INIT_FAILED` (one init per guest instance).
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn actr_init(
                vtable: *const $crate::guest::vtable::HostVTable,
                init_ptr: *const u8,
                init_len: usize,
            ) -> i32 {
                if vtable.is_null() {
                    return $crate::guest::abi::code::INIT_FAILED;
                }

                let init_bytes = if init_ptr.is_null() || init_len == 0 {
                    &[][..]
                } else {
                    unsafe { std::slice::from_raw_parts(init_ptr, init_len) }
                };

                // TODO: `actr_init` currently only validates that `InitPayloadV1`
                // is decodable. The payload fields themselves are not yet
                // consumed by the guest runtime on the dynclib path. This is a
                // legacy gap carried forward from the previous init model.
                if $crate::guest::abi::decode_message::<$crate::guest::abi::InitPayloadV1>(
                    init_bytes,
                )
                .is_err()
                {
                    return $crate::guest::abi::code::PROTOCOL_ERROR;
                }

                let workload: $workload_type = $init_expr;
                unsafe {
                    if __ACTR_WORKLOAD.is_some() {
                        return $crate::guest::abi::code::INIT_FAILED;
                    }
                    __ACTR_VTABLE = Some(vtable);
                    __ACTR_WORKLOAD = Some(workload);
                }
                $crate::guest::abi::code::SUCCESS
            }

            /// Handle one runtime ABI frame.
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

                // Read runtime frame
                if req_ptr.is_null() {
                    return $crate::guest::abi::code::PROTOCOL_ERROR;
                }
                let req_bytes = unsafe { std::slice::from_raw_parts(req_ptr, req_len) };

                let frame = match $crate::guest::abi::decode_message::<
                    $crate::guest::abi::AbiFrame,
                >(req_bytes) {
                    Ok(f) => f,
                    Err(_) => return $crate::guest::abi::code::PROTOCOL_ERROR,
                };

                if frame.op != $crate::guest::abi::op::GUEST_HANDLE {
                    return $crate::guest::abi::code::UNSUPPORTED_OP;
                }

                let handle = match <$crate::guest::abi::GuestHandleV1 as $crate::guest::abi::AbiPayload>::decode_payload(&frame.payload) {
                    Ok(handle) => handle,
                    Err(_) => return $crate::guest::abi::code::PROTOCOL_ERROR,
                };

                let envelope = match actr_protocol::RpcEnvelope::decode(handle.rpc_envelope.as_slice()) {
                    Ok(e) => e,
                    Err(_) => return $crate::guest::abi::code::PROTOCOL_ERROR,
                };

                let ctx = match unsafe {
                    $crate::guest::dynclib::context::DynclibContext::from_invocation(vtable, handle.ctx)
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
                // Here we use the same single-threaded poll strategy as the old WASM path:
                // All host callbacks (vtable function pointers) are synchronous, Future
                // completes in one poll.
                let resp_result = {
                    let fut = Dispatcher::dispatch(workload, envelope, &ctx);
                    let waker = std::task::Waker::noop();
                    let mut cx = std::task::Context::from_waker(waker);
                    let mut pinned = std::pin::pin!(fut);
                    match pinned.as_mut().poll(&mut cx) {
                        std::task::Poll::Ready(v) => v,
                        std::task::Poll::Pending => {
                            return $crate::guest::abi::code::HANDLE_FAILED;
                        }
                    }
                };

                let resp_bytes = match resp_result {
                    Ok(b) => match $crate::guest::abi::success_reply(b.to_vec()) {
                        Ok(bytes) => bytes,
                        Err(code) => return code,
                    },
                    Err(err) => match $crate::guest::abi::error_reply(
                        $crate::guest::abi::code::HANDLE_FAILED,
                        err.to_string().into_bytes(),
                    ) {
                        Ok(bytes) => bytes,
                        Err(code) => return code,
                    },
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
