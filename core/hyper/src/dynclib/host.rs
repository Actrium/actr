//! DynclibHost / DynClibWorkload — native shared-library actor execution engine
//!
//! Loads a cdylib SO/dylib/DLL and resolves the standard ABI symbols:
//!
//! - `actr_init(vtable, init_ptr, init_len) -> i32`
//! - `actr_handle(req_ptr, req_len, resp_out, resp_len_out) -> i32`
//! - `actr_free_response(ptr, len)`
//!
//! The guest library calls back into the host through a `HostVTable` passed at
//! init time. VTable trampolines bridge the synchronous C ABI with the async
//! Rust host ABI bridge via thread-local storage and `tokio::runtime::Handle`.
//!
//! Each loaded shared-library image currently supports exactly one logical actor
//! instance. If the host wants to run two actors from the same dynclib package,
//! it must load two independent library images and keep dispatch serialized per
//! workload.
//!
//! TODO: Decide whether Dynclib should eventually support a "one host loads once,
//! many workloads instantiate independently" model like WASM. That requires an
//! explicit instance design at the ABI/runtime boundary instead of relying on
//! module-global guest state.

use std::cell::RefCell;
use std::path::Path;
use std::ptr;

use actr_framework::guest::abi::{self as guest_abi, AbiReply, InitPayloadV1};
use libloading::Library;

/// Wrapper around a raw pointer that is `Send`.
///
/// Safety: the caller must guarantee that the pointed-to value outlives the
/// `SendPtr` and that no data races occur (i.e. exclusive or shared access
/// rules are upheld externally).
struct SendPtr<T>(*const T);

// Safety: we ensure the pointed-to host ABI closure outlives the
// `spawn_blocking` task by awaiting the task's completion before the
// reference goes out of scope.
unsafe impl<T> Send for SendPtr<T> {}

impl<T> SendPtr<T> {
    fn as_ptr(&self) -> *const T {
        self.0
    }
}

use actr_framework::guest::vtable::HostVTable;

use crate::workload::{
    HostAbiFn, HostOperation, HostOperationResult, InvocationContext, encode_guest_handle_request,
};

use super::error::{DynclibError, DynclibResult};

// ─────────────────────────────────────────────────────────────────────────────
// C ABI function signatures expected from the guest SO
// ─────────────────────────────────────────────────────────────────────────────

/// `actr_init(vtable: *const HostVTable, init_ptr: *const u8, init_len: usize) -> i32`
type InitFn = unsafe extern "C" fn(
    vtable: *const HostVTable,
    init_payload: *const u8,
    init_len: usize,
) -> i32;

/// `actr_handle(req_ptr: *const u8, req_len: usize, resp_out: *mut *mut u8, resp_len_out: *mut usize) -> i32`
type HandleFn = unsafe extern "C" fn(
    req: *const u8,
    req_len: usize,
    resp_out: *mut *mut u8,
    resp_len_out: *mut usize,
) -> i32;

/// `actr_free_response(ptr: *mut u8, len: usize)`
type FreeResponseFn = unsafe extern "C" fn(ptr: *mut u8, len: usize);

// ─────────────────────────────────────────────────────────────────────────────
// Thread-local state for VTable trampolines
// ─────────────────────────────────────────────────────────────────────────────

thread_local! {
    /// Pointer to the active `HostAbiFn` for the current dispatch.
    static CURRENT_EXECUTOR: RefCell<Option<*const HostAbiFn>> = const { RefCell::new(None) };

    /// Tokio runtime handle used by trampolines to block on async futures.
    static TOKIO_HANDLE: RefCell<Option<tokio::runtime::Handle>> = const { RefCell::new(None) };
}

/// Install thread-local state before calling into the guest SO.
fn install_thread_locals(executor: *const HostAbiFn, handle: tokio::runtime::Handle) {
    CURRENT_EXECUTOR.with(|cell| *cell.borrow_mut() = Some(executor));
    TOKIO_HANDLE.with(|cell| *cell.borrow_mut() = Some(handle));
}

/// Clear thread-local state after the guest SO returns.
fn clear_thread_locals() {
    CURRENT_EXECUTOR.with(|cell| *cell.borrow_mut() = None);
    TOKIO_HANDLE.with(|cell| *cell.borrow_mut() = None);
}

// ─────────────────────────────────────────────────────────────────────────────
// VTable trampoline implementations
// ─────────────────────────────────────────────────────────────────────────────

/// Allocate a buffer and copy `data` into it, writing the pointer and length
/// into the caller-provided out parameters.
///
/// # Safety
/// `out_ptr` and `out_len` must be valid, aligned, non-null pointers.
unsafe fn host_alloc_and_write(data: &[u8], out_ptr: *mut *mut u8, out_len: *mut usize) {
    let len = data.len();
    let buf = if len > 0 {
        let layout = std::alloc::Layout::from_size_align(len, 1).expect("invalid layout");
        // Safety: layout has non-zero size (len > 0).
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        // Safety: ptr is valid for `len` bytes; data.len() == len.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, len) };
        ptr
    } else {
        ptr::null_mut()
    };
    // Safety: caller guarantees out_ptr/out_len are valid.
    unsafe {
        *out_ptr = buf;
        *out_len = len;
    }
}

/// Execute a host operation through the thread-local `HostAbiFn`.
///
/// This blocks the current (blocking) thread by calling `Handle::block_on`
/// on the tokio runtime handle saved in thread-local storage.
///
/// Returns the host operation result or an error code if the thread-local state is missing.
fn trampoline_execute(pending: HostOperation) -> HostOperationResult {
    let maybe_result = TOKIO_HANDLE.with(|h_cell| {
        let h_borrow = h_cell.borrow();
        let handle = match h_borrow.as_ref() {
            Some(h) => h,
            None => {
                tracing::error!("dynclib trampoline: TOKIO_HANDLE not set");
                return None;
            }
        };

        CURRENT_EXECUTOR.with(|e_cell| {
            let e_borrow = e_cell.borrow();
            let executor_ptr = match *e_borrow {
                Some(p) => p,
                None => {
                    tracing::error!("dynclib trampoline: CURRENT_EXECUTOR not set");
                    return None;
                }
            };

            // Safety: the pointer is valid for the duration of the dispatch
            // (set in `DynclibInstance::handle` and cleared after the guest
            // call returns).
            let executor: &HostAbiFn = unsafe { &*executor_ptr };
            let future = executor(pending);
            // Block on the async future. This is safe because we are running
            // inside `spawn_blocking`, not on a tokio worker thread.
            Some(handle.block_on(future))
        })
    });
    maybe_result.unwrap_or(HostOperationResult::Error(guest_abi::code::GENERIC_ERROR))
}

/// Read bytes from raw pointer + length, returning an empty Vec on null/zero.
///
/// # Safety
/// If `ptr` is non-null, `ptr` must be valid for reads of `len` bytes.
unsafe fn read_raw_bytes(ptr: *const u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // Safety: caller guarantees ptr is valid for `len` bytes.
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

use crate::workload::decode_host_operation;

unsafe extern "C" fn vtable_invoke(
    frame_ptr: *const u8,
    frame_len: usize,
    resp_ptr_out: *mut *mut u8,
    resp_len_out: *mut usize,
) -> i32 {
    if resp_ptr_out.is_null() || resp_len_out.is_null() {
        return guest_abi::code::PROTOCOL_ERROR;
    }

    let frame_bytes = unsafe { read_raw_bytes(frame_ptr, frame_len) };
    let frame = match guest_abi::decode_message::<guest_abi::AbiFrame>(&frame_bytes) {
        Ok(frame) => frame,
        Err(code) => return code,
    };

    let pending = match decode_host_operation(frame) {
        Ok(pending) => pending,
        Err(code) => return code,
    };

    let reply = match trampoline_execute(pending) {
        HostOperationResult::Bytes(bytes) => AbiReply {
            abi_version: guest_abi::version::V1,
            status: guest_abi::code::SUCCESS,
            payload: bytes,
        },
        HostOperationResult::Done => AbiReply {
            abi_version: guest_abi::version::V1,
            status: guest_abi::code::SUCCESS,
            payload: Vec::new(),
        },
        HostOperationResult::Error(code) => AbiReply {
            abi_version: guest_abi::version::V1,
            status: code,
            payload: Vec::new(),
        },
    };

    let reply_bytes = match guest_abi::encode_message(&reply) {
        Ok(reply_bytes) => reply_bytes,
        Err(code) => return code,
    };

    unsafe { host_alloc_and_write(&reply_bytes, resp_ptr_out, resp_len_out) };
    guest_abi::code::SUCCESS
}

// ── VTable::free_host_buf ───────────────────────────────────────────────────

unsafe extern "C" fn vtable_free_host_buf(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    let layout = std::alloc::Layout::from_size_align(len, 1).expect("invalid layout in free");
    // Safety: the buffer was allocated by `host_alloc_and_write` using
    // `std::alloc::alloc` with Layout::from_size_align(len, 1). The guest
    // must not use the pointer after calling this function.
    unsafe { std::alloc::dealloc(ptr, layout) };
}

/// Static VTable instance with all trampolines wired up.
static HOST_VTABLE: HostVTable = HostVTable {
    invoke: vtable_invoke,
    free_host_buf: vtable_free_host_buf,
};

// ─────────────────────────────────────────────────────────────────────────────
// DynclibHost
// ─────────────────────────────────────────────────────────────────────────────

/// Native shared-library host engine.
///
/// Loads and holds a single `.so` / `.dylib` / `.dll` image. Resolves ABI
/// symbols once at load time.
///
/// Under the current guest ABI, a loaded dynclib image supports only one
/// successful `actr_init` because guest state is module-global and no instance
/// handle is exposed back to the host. To create multiple independent
/// `DynClibWorkload`s today, Hyper must load multiple library images.
///
/// TODO: Revisit this contract if Dynclib gains a real per-instance ABI.
pub struct DynclibHost {
    /// Loaded shared library handle. Must outlive all resolved function pointers.
    _library: Library,
    init_fn: InitFn,
    handle_fn: HandleFn,
    free_response_fn: FreeResponseFn,
}

impl std::fmt::Debug for DynclibHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynclibHost").finish_non_exhaustive()
    }
}

// Safety: The Library handle and resolved function pointers are safe to send
// across threads. The resolved symbols point into memory-mapped shared library
// code which is process-global and immutable.
unsafe impl Send for DynclibHost {}
unsafe impl Sync for DynclibHost {}

impl DynclibHost {
    /// Verify package signature, then load the shared library.
    ///
    /// Supports both `.actr` ZIP packages and legacy binaries with embedded manifests.
    /// For `.actr` packages, the native binary is extracted to a temporary file and loaded from there.
    pub fn load_verified(
        path: impl AsRef<Path>,
        verifier: &crate::verify::PackageVerifier,
    ) -> DynclibResult<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| {
            DynclibError::LoadFailed(format!(
                "failed to read binary for verification: {}: {e}",
                path.display()
            ))
        })?;
        let manifest = verifier.verify(&bytes)?;
        tracing::info!(
            manufacturer = %manifest.manufacturer,
            actr_name = %manifest.actr_name,
            version = %manifest.version,
            "dynclib package signature verified, proceeding to load"
        );

        // .actr ZIP package: extract binary to temp file and load
        if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
            let binary_bytes = actr_pack::load_binary(&bytes).map_err(|e| {
                DynclibError::LoadFailed(format!(
                    "failed to extract binary from .actr package: {e}"
                ))
            })?;
            let tmp_dir = path.parent().unwrap_or(Path::new("."));
            let tmp_path = tmp_dir.join(format!(".actr-tmp-{}", manifest.actr_name));
            std::fs::write(&tmp_path, &binary_bytes).map_err(|e| {
                DynclibError::LoadFailed(format!("failed to write extracted binary: {e}"))
            })?;
            let result = Self::load(&tmp_path);
            let _ = std::fs::remove_file(&tmp_path);
            result
        } else {
            Self::load(path)
        }
    }

    /// Load a shared library from the given filesystem path.
    ///
    /// Resolves the required ABI symbols (`actr_init`, `actr_handle`,
    /// `actr_free_response`). Returns an error if any symbol is missing.
    pub fn load(path: impl AsRef<Path>) -> DynclibResult<Self> {
        let path = path.as_ref();
        tracing::info!(path = %path.display(), "loading dynclib actor");

        // Safety: loading a shared library executes its static initialisers,
        // which is inherently unsafe. The caller must ensure the library is
        // trusted (e.g. verified by Hyper's package verification).
        let library = unsafe {
            Library::new(path)
                .map_err(|e| DynclibError::LoadFailed(format!("{}: {e}", path.display())))?
        };

        // Safety: we resolve raw symbol pointers and transmute them to typed
        // function pointers. The caller must guarantee that the SO exports
        // these symbols with the correct C ABI signatures.
        let init_fn: InitFn = unsafe {
            let sym =
                library
                    .get::<InitFn>(b"actr_init\0")
                    .map_err(|e| DynclibError::MissingSymbol {
                        symbol: "actr_init".into(),
                        detail: e.to_string(),
                    })?;
            *sym
        };

        let handle_fn: HandleFn = unsafe {
            let sym = library.get::<HandleFn>(b"actr_handle\0").map_err(|e| {
                DynclibError::MissingSymbol {
                    symbol: "actr_handle".into(),
                    detail: e.to_string(),
                }
            })?;
            *sym
        };

        let free_response_fn: FreeResponseFn = unsafe {
            let sym = library
                .get::<FreeResponseFn>(b"actr_free_response\0")
                .map_err(|e| DynclibError::MissingSymbol {
                    symbol: "actr_free_response".into(),
                    detail: e.to_string(),
                })?;
            *sym
        };

        tracing::info!(path = %path.display(), "dynclib symbols resolved successfully");

        Ok(Self {
            _library: library,
            init_fn,
            handle_fn,
            free_response_fn,
        })
    }

    /// Initialise an actor instance inside the loaded library.
    ///
    /// Calls the guest's `actr_init(vtable, init_ptr, init_len)`.
    pub fn instantiate(&self, init_payload: &InitPayloadV1) -> DynclibResult<DynclibInstance> {
        let init_bytes = guest_abi::encode_message(init_payload).map_err(|code| {
            DynclibError::DispatchFailed(format!("init payload encode failed: {code}"))
        })?;
        let init_ptr = if init_bytes.is_empty() {
            ptr::null()
        } else {
            init_bytes.as_ptr()
        };

        // Safety: `actr_init` is a C function resolved from the shared
        // library. `HOST_VTABLE` is a static with stable address. `init_ptr`
        // and `init_bytes.len()` describe a valid byte slice (or null/0).
        let result = unsafe { (self.init_fn)(&HOST_VTABLE, init_ptr, init_bytes.len()) };

        if result != 0 {
            tracing::error!(code = result, "actr_init failed");
            return Err(DynclibError::InitFailed(result));
        }

        tracing::info!("dynclib actor initialised successfully");

        Ok(DynclibInstance {
            handle_fn: self.handle_fn,
            free_response_fn: self.free_response_fn,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DynclibInstance
// ─────────────────────────────────────────────────────────────────────────────

/// Per-actor instance backed by a native shared library.
///
/// Holds cached function pointers for `actr_handle` and `actr_free_response`.
/// `actr_init` initializes exactly one logical actor state inside this instance.
/// **Not `Sync`**: callers must serialise access (e.g. via `Mutex<DynClibWorkload>`)
/// and must not enter `actr_handle` concurrently for the same instance.
pub struct DynclibInstance {
    handle_fn: HandleFn,
    free_response_fn: FreeResponseFn,
}

impl std::fmt::Debug for DynclibInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynclibInstance").finish_non_exhaustive()
    }
}

// Safety: function pointers reference process-global SO code.
unsafe impl Send for DynclibInstance {}

/// Executor wrapper that keeps the loaded library alive for the lifetime of the actor instance.
///
/// Field order matters: Rust drops fields in declaration order, so `instance`
/// (which holds raw function pointers into the loaded library) must be dropped
/// before `_host` (which unloads the library), and `_host` before `_temp_file`
/// (which deletes the on-disk shared object).
#[derive(Debug)]
pub struct DynClibWorkload {
    instance: DynclibInstance,
    _host: DynclibHost,
    _temp_file: Option<tempfile::NamedTempFile>,
}

impl DynClibWorkload {
    pub(crate) fn with_temp_file(
        host: DynclibHost,
        instance: DynclibInstance,
        temp_file: tempfile::NamedTempFile,
    ) -> Self {
        Self {
            instance,
            _host: host,
            _temp_file: Some(temp_file),
        }
    }
}

impl DynclibInstance {
    /// Dispatch a request through the guest actor.
    ///
    /// This method:
    /// 1. Installs thread-local state (executor, context, tokio handle)
    /// 2. Calls the guest's `actr_handle` on a blocking thread
    /// 3. Copies the response, frees the guest-allocated buffer
    /// 4. Clears thread-local state
    ///
    /// The guest SO may call VTable trampolines synchronously during
    /// `actr_handle`. Those trampolines use `Handle::block_on` to execute the
    /// async `call_executor` — this is safe because `actr_handle` runs inside
    /// `spawn_blocking` (off the tokio worker pool).
    pub async fn handle(
        &mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        call_executor: &HostAbiFn,
    ) -> DynclibResult<Vec<u8>> {
        let handle_fn = self.handle_fn;
        let free_response_fn = self.free_response_fn;
        let request_owned = encode_guest_handle_request(request_bytes, ctx).map_err(|code| {
            DynclibError::DispatchFailed(format!("guest handle frame serialization failed: {code}"))
        })?;

        // Obtain a handle to the current tokio runtime so trampolines can
        // block on async futures from the blocking thread.
        let rt_handle = tokio::runtime::Handle::current();

        // Erase lifetime: the pointer is valid for the duration of the
        // `spawn_blocking` task because we await its completion below.
        let executor_ptr = SendPtr(call_executor as *const HostAbiFn);

        let result = tokio::task::spawn_blocking(move || {
            // Install thread-local state for VTable trampolines.
            install_thread_locals(executor_ptr.as_ptr(), rt_handle);

            // Prepare output pointers.
            let mut resp_ptr: *mut u8 = ptr::null_mut();
            let mut resp_len: usize = 0;

            // Safety: `handle_fn` is a C function from the loaded SO.
            // `request_owned` is a valid Vec<u8> and `as_ptr()`/`len()` describe
            // a valid slice. `resp_ptr` and `resp_len` are stack-local variables
            // whose addresses are valid for the duration of the call.
            let code = unsafe {
                (handle_fn)(
                    request_owned.as_ptr(),
                    request_owned.len(),
                    &mut resp_ptr,
                    &mut resp_len,
                )
            };

            // Copy response bytes before freeing the guest buffer.
            let response = if !resp_ptr.is_null() && resp_len > 0 {
                // Safety: the guest set resp_ptr/resp_len to describe a valid
                // allocation. We copy before calling free_response_fn.
                let data = unsafe { std::slice::from_raw_parts(resp_ptr, resp_len).to_vec() };

                // Safety: free the guest-allocated response buffer with the
                // guest's own free function.
                unsafe { (free_response_fn)(resp_ptr, resp_len) };

                data
            } else {
                Vec::new()
            };

            // Clear thread-local state.
            clear_thread_locals();

            if code != 0 {
                tracing::warn!(code, "actr_handle returned error");
                return Err(DynclibError::DispatchFailed(format!(
                    "actr_handle returned error code {code}"
                )));
            }

            tracing::debug!(
                req_bytes = request_owned.len(),
                resp_bytes = response.len(),
                "actr_handle completed"
            );

            Ok(response)
        })
        .await
        .map_err(|e| DynclibError::DispatchFailed(format!("spawn_blocking panicked: {e}")))??;

        Ok(result)
    }
}

impl DynClibWorkload {
    pub async fn handle(
        &mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        call_executor: &HostAbiFn,
    ) -> DynclibResult<Vec<u8>> {
        self.instance
            .handle(request_bytes, ctx, call_executor)
            .await
    }
}
