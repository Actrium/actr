//! WasmHost — Wasmtime host engine
//!
//! Implements the full lifecycle from WASM byte loading, compilation, instantiation to
//! message dispatch. A single `WasmHost` corresponds to one WASM module (compiled once),
//! from which multiple `WasmWorkload`s can be derived.
//!
//! Each `WasmWorkload` is one logical actor instance. If the host wants to run two
//! actors of the same WASM type, it instantiates the module twice and keeps the
//! resulting instances isolated.
//!
//! # Asyncify driver
//!
//! `dispatch()` uses the asyncify unwind/rewind protocol:
//! 1. Call `actr_handle`
//! 2. If WASM triggers a host import (e.g. `actr_host_call`), the import initiates unwind and suspends
//! 3. Drive loop detects Unwinding, executes real async IO
//! 4. Stores result in `HostData`, initiates rewind, re-calls `actr_handle`
//! 5. Import returns real result in Rewinding mode, WASM continues execution

use actr_protocol::prost::Message as ProstMessage;
use wasmtime::{Caller, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::wasm::error::{WasmError, WasmResult};
use crate::workload::{
    HostOperation, HostOperationResult, InvocationContext, encode_guest_handle_request,
};
use actr_framework::guest::abi::{self as guest_abi, AbiReply, InitPayloadV1};

use super::abi;

/// Re-bind the canonical error code module for concise usage within this file.
use actr_framework::guest::abi::code as abi_code;

// ─────────────────────────────────────────────────────────────────────────────
// HostData — runtime state stored in Store
// ─────────────────────────────────────────────────────────────────────────────

/// Asyncify state machine
#[derive(Debug, Clone, PartialEq, Default)]
enum AsyncifyMode {
    #[default]
    Normal,
    Unwinding,
    Rewinding,
}

/// Wasmtime Store internal data
#[derive(Debug, Default)]
struct HostData {
    // ── asyncify protocol ─────────────────────────────────────────────────
    asyncify_mode: AsyncifyMode,
    asyncify_data_ptr: i32,
    // ── current invocation context for legacy getter imports ──────────────
    current_invocation: Option<InvocationContext>,
    // ── pending IO saved when host import suspends ────────────────────────
    pending_call: Option<HostOperation>,
    // ── drive loop writes IO result here, host import reads during rewind ─
    io_result: Option<HostOperationResult>,
}

// asyncify data buffer layout (fixed address, WASM page 0)
const ASYNCIFY_DATA_PTR: i32 = 0x8000; // 32 KB
const ASYNCIFY_STACK_START: i32 = ASYNCIFY_DATA_PTR + 8;
const ASYNCIFY_STACK_END: i32 = ASYNCIFY_DATA_PTR + 0x1000; // +4 KB

// ─────────────────────────────────────────────────────────────────────────────
// WasmHost
// ─────────────────────────────────────────────────────────────────────────────

/// WASM host engine
///
/// Compiles and holds a WASM module; the same module can be instantiated multiple times
/// (one instance per actor). Compilation is CPU-intensive and should be done once,
/// then the `WasmHost` is reused.
pub struct WasmHost {
    engine: Engine,
    module: Module,
}

impl std::fmt::Debug for WasmHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHost").finish_non_exhaustive()
    }
}

impl WasmHost {
    /// Compile a WASM module from bytes (CPU-intensive, recommend calling in `spawn_blocking`)
    pub fn compile(wasm_bytes: &[u8]) -> WasmResult<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| WasmError::LoadFailed(format!("module compilation failed: {e}")))?;

        tracing::info!(wasm_bytes = wasm_bytes.len(), "WASM module compiled");
        Ok(Self { engine, module })
    }

    /// Verify package signature, then compile the WASM module.
    ///
    /// Supports both `.actr` ZIP packages and legacy WASM binaries with embedded manifests.
    /// For `.actr` packages, the binary is extracted after verification.
    pub fn compile_verified(
        package_bytes: &[u8],
        verifier: &crate::verify::PackageVerifier,
    ) -> WasmResult<Self> {
        let manifest = verifier.verify(package_bytes)?;
        tracing::info!(
            manufacturer = %manifest.manufacturer,
            actr_name = %manifest.actr_name,
            version = %manifest.version,
            "package signature verified, proceeding to compile"
        );

        // .actr ZIP package: extract binary from the archive
        if package_bytes.len() >= 4 && &package_bytes[0..4] == b"PK\x03\x04" {
            let wasm_bytes = actr_pack::load_binary(package_bytes).map_err(|e| {
                WasmError::LoadFailed(format!("failed to extract binary from .actr package: {e}"))
            })?;
            Self::compile(&wasm_bytes)
        } else {
            // Legacy: WASM bytes with embedded manifest
            Self::compile(package_bytes)
        }
    }

    /// Instantiate the WASM module, register all host imports, return an executable `WasmWorkload`
    pub fn instantiate(&self) -> WasmResult<WasmWorkload> {
        let mut linker = Linker::<HostData>::new(&self.engine);
        register_host_imports(&mut linker)?;
        let legacy_handle_payload = uses_legacy_handle_payload(&self.module);

        let mut store = Store::new(&self.engine, HostData::default());

        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| WasmError::LoadFailed(format!("module instantiation failed: {e}")))?;

        // initialize asyncify data buffer
        init_asyncify_data(&instance, &mut store);

        let actr_init = resolve_func::<(i32, i32), i32>(&instance, &mut store, abi::EXPORT_INIT)?;
        let actr_handle =
            resolve_func::<(i32, i32, i32, i32), i32>(&instance, &mut store, abi::EXPORT_HANDLE)?;
        let actr_alloc = resolve_func::<i32, i32>(&instance, &mut store, abi::EXPORT_ALLOC)?;
        let actr_free = resolve_func::<(i32, i32), ()>(&instance, &mut store, abi::EXPORT_FREE)?;
        let memory = instance
            .get_memory(&mut store, abi::EXPORT_MEMORY)
            .ok_or_else(|| {
                WasmError::LoadFailed(
                    "WASM module does not export linear memory 'memory'".to_string(),
                )
            })?;

        // asyncify control functions (injected into WASM binary by wasm-opt --asyncify)
        let asyncify_stop_unwind =
            resolve_func::<(), ()>(&instance, &mut store, "asyncify_stop_unwind")?;
        let asyncify_start_rewind =
            resolve_func::<i32, ()>(&instance, &mut store, "asyncify_start_rewind")?;

        tracing::info!("WASM instantiation succeeded, all ABI export functions verified");

        Ok(WasmWorkload {
            store,
            _instance: instance,
            actr_init,
            actr_handle,
            actr_alloc,
            actr_free,
            memory,
            asyncify_stop_unwind,
            asyncify_start_rewind,
            legacy_handle_payload,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Host imports registration
// ─────────────────────────────────────────────────────────────────────────────

fn register_host_imports(linker: &mut Linker<HostData>) -> WasmResult<()> {
    // Register minimal WASI stubs for wasi_snapshot_preview1
    // These are no-op implementations to satisfy WASM imports
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_get",
            |_: i32, _: i32| -> i32 { 0 },
        )
        .map_err(|e| WasmError::LoadFailed(format!("failed to register environ_get: {e}")))?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_sizes_get",
            |_: i32, _: i32| -> i32 { 0 },
        )
        .map_err(|e| WasmError::LoadFailed(format!("failed to register environ_sizes_get: {e}")))?;

    linker
        .func_wrap("wasi_snapshot_preview1", "proc_exit", |_: i32| {})
        .map_err(|e| WasmError::LoadFailed(format!("failed to register proc_exit: {e}")))?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            |_: i32, _: i32, _: i32, _: i32| -> i32 { 0 },
        )
        .map_err(|e| WasmError::LoadFailed(format!("failed to register fd_write: {e}")))?;

    linker
        .func_wrap("wasi_snapshot_preview1", "fd_close", |_: i32| -> i32 { 0 })
        .map_err(|e| WasmError::LoadFailed(format!("failed to register fd_close: {e}")))?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_seek",
            |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 },
        )
        .map_err(|e| WasmError::LoadFailed(format!("failed to register fd_seek: {e}")))?;

    // Register actr-specific imports
    linker
        .func_wrap(
            "env",
            "actr_host_self_id",
            |mut caller: Caller<HostData>, buf_ptr: i32, buf_cap: i32| -> i32 {
                let Some(ctx) = caller.data().current_invocation.as_ref() else {
                    return abi_code::GENERIC_ERROR;
                };
                let bytes = ctx.self_id.encode_to_vec();
                write_legacy_context_bytes(&mut caller, &bytes, buf_ptr, buf_cap)
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("failed to register actr_host_self_id: {e}")))?;

    linker
        .func_wrap(
            "env",
            "actr_host_caller_id",
            |mut caller: Caller<HostData>, buf_ptr: i32, buf_cap: i32| -> i32 {
                let Some(ctx) = caller.data().current_invocation.as_ref() else {
                    return abi_code::GENERIC_ERROR;
                };
                let bytes = ctx
                    .caller_id
                    .as_ref()
                    .map(ProstMessage::encode_to_vec)
                    .unwrap_or_default();
                write_legacy_context_bytes(&mut caller, &bytes, buf_ptr, buf_cap)
            },
        )
        .map_err(|e| {
            WasmError::LoadFailed(format!("failed to register actr_host_caller_id: {e}"))
        })?;

    linker
        .func_wrap(
            "env",
            "actr_host_request_id",
            |mut caller: Caller<HostData>, buf_ptr: i32, buf_cap: i32| -> i32 {
                let Some(ctx) = caller.data().current_invocation.as_ref() else {
                    return abi_code::GENERIC_ERROR;
                };
                let bytes = ctx.request_id.as_bytes().to_vec();
                write_legacy_context_bytes(&mut caller, &bytes, buf_ptr, buf_cap)
            },
        )
        .map_err(|e| {
            WasmError::LoadFailed(format!("failed to register actr_host_request_id: {e}"))
        })?;

    linker
        .func_wrap(
            "env",
            "actr_host_invoke",
            |mut caller: Caller<HostData>,
             frame_ptr: i32,
             frame_len: i32,
             reply_buf_ptr: i32,
             reply_buf_cap: i32,
             reply_len_out: i32|
             -> i32 {
                match caller.data().asyncify_mode.clone() {
                    AsyncifyMode::Normal => {
                        let frame_bytes = read_bytes_from_wasm(&mut caller, frame_ptr, frame_len);
                        let frame =
                            match guest_abi::decode_message::<guest_abi::AbiFrame>(&frame_bytes) {
                                Ok(frame) => frame,
                                Err(code) => return code,
                            };

                        let pending = match decode_host_operation(frame) {
                            Ok(pending) => pending,
                            Err(code) => return code,
                        };

                        caller.data_mut().pending_call = Some(pending);
                        caller.data_mut().asyncify_mode = AsyncifyMode::Unwinding;
                        trigger_unwind(&mut caller);
                        abi_code::SUCCESS
                    }
                    AsyncifyMode::Rewinding => {
                        let reply_bytes = match caller.data_mut().io_result.take() {
                            Some(HostOperationResult::Bytes(bytes)) => {
                                match guest_abi::encode_message(&AbiReply {
                                    abi_version: guest_abi::version::V1,
                                    status: guest_abi::code::SUCCESS,
                                    payload: bytes,
                                }) {
                                    Ok(reply) => reply,
                                    Err(code) => return code,
                                }
                            }
                            Some(HostOperationResult::Done) => {
                                match guest_abi::encode_message(&AbiReply {
                                    abi_version: guest_abi::version::V1,
                                    status: guest_abi::code::SUCCESS,
                                    payload: Vec::new(),
                                }) {
                                    Ok(reply) => reply,
                                    Err(code) => return code,
                                }
                            }
                            Some(HostOperationResult::Error(code)) => {
                                match guest_abi::encode_message(&AbiReply {
                                    abi_version: guest_abi::version::V1,
                                    status: code,
                                    payload: Vec::new(),
                                }) {
                                    Ok(reply) => reply,
                                    Err(code) => return code,
                                }
                            }
                            None => return abi_code::GENERIC_ERROR,
                        };

                        let reply_len = reply_bytes.len();
                        let reply_len_i32 = match i32::try_from(reply_len) {
                            Ok(len) => len,
                            Err(_) => return guest_abi::code::GENERIC_ERROR,
                        };

                        write_i32_to_wasm(&mut caller, reply_len_out, reply_len_i32);
                        if reply_len > reply_buf_cap.max(0) as usize {
                            caller.data_mut().asyncify_mode = AsyncifyMode::Normal;
                            trigger_stop_rewind(&mut caller);
                            return guest_abi::code::BUFFER_TOO_SMALL;
                        }

                        let written =
                            write_to_wasm(&mut caller, &reply_bytes, reply_buf_ptr, reply_buf_cap);
                        write_i32_to_wasm(&mut caller, reply_len_out, written);
                        caller.data_mut().asyncify_mode = AsyncifyMode::Normal;
                        trigger_stop_rewind(&mut caller);
                        abi_code::SUCCESS
                    }
                    AsyncifyMode::Unwinding => abi_code::SUCCESS,
                }
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("failed to register actr_host_invoke: {e}")))?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// WasmWorkload
// ─────────────────────────────────────────────────────────────────────────────

/// Single WASM actor instance
///
/// Wraps a Wasmtime `Store<HostData>` and cached export function handles.
/// `actr_init` initializes exactly one logical actor state inside this instance.
/// **Not `Sync`**: the caller is responsible for concurrency protection
/// (typically `Mutex<WasmWorkload>`), and must not drive `handle()` concurrently
/// on the same instance.
pub struct WasmWorkload {
    store: Store<HostData>,
    _instance: Instance,
    actr_init: TypedFunc<(i32, i32), i32>,
    actr_handle: TypedFunc<(i32, i32, i32, i32), i32>,
    actr_alloc: TypedFunc<i32, i32>,
    actr_free: TypedFunc<(i32, i32), ()>,
    memory: Memory,
    asyncify_stop_unwind: TypedFunc<(), ()>,
    asyncify_start_rewind: TypedFunc<i32, ()>,
    legacy_handle_payload: bool,
}

impl std::fmt::Debug for WasmWorkload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmWorkload").finish_non_exhaustive()
    }
}

impl WasmWorkload {
    /// Initialize the WASM actor (calls `actr_init`)
    pub fn init(&mut self, init_payload: &InitPayloadV1) -> WasmResult<()> {
        let init_bytes = guest_abi::encode_message(init_payload)
            .map_err(|code| WasmError::InitFailed(format!("init payload encode failed: {code}")))?;

        let ptr = self.wasm_write(&init_bytes)?;
        let len = init_bytes.len() as i32;

        let result = self
            .actr_init
            .call(&mut self.store, (ptr, len))
            .map_err(|e| WasmError::InitFailed(format!("actr_init call failed: {e}")))?;

        self.wasm_free(ptr, len)?;

        if result != abi_code::SUCCESS {
            return Err(WasmError::InitFailed(format!(
                "actr_init returned error code {result} ({})",
                abi::describe_error_code(result)
            )));
        }

        tracing::info!("WASM actor initialized");
        Ok(())
    }

    /// Handle one RPC request, using the asyncify drive loop to service outbound IO.
    ///
    /// `ctx`: context data for this call (self_id, caller_id, request_id)
    /// `call_executor`: handles outbound calls initiated by the guest
    ///
    /// Note: `call_executor` needs to perform real async IO, but Wasmtime host imports are synchronous.
    /// The asyncify protocol exposes the "IO needed" moment to the drive loop (this function),
    /// where `call_executor` (async) can be called.
    pub async fn handle<F, Fut>(
        &mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        call_executor: F,
    ) -> WasmResult<Vec<u8>>
    where
        F: Fn(HostOperation) -> Fut,
        Fut: std::future::Future<Output = HostOperationResult>,
    {
        // Reset asyncify data buffer before each dispatch
        // (previous unwind may have written to the buffer; must reset the write pointer before a new handle call)
        reset_asyncify_data(&mut self.store, &self.memory);

        self.store.data_mut().asyncify_mode = AsyncifyMode::Normal;

        self.store.data_mut().current_invocation = Some(ctx.clone());

        let request_bytes = if self.legacy_handle_payload {
            request_bytes.to_vec()
        } else {
            encode_guest_handle_request(request_bytes, ctx).map_err(|code| {
                WasmError::ExecutionFailed(format!(
                    "guest handle frame serialization failed: {code}"
                ))
            })?
        };

        // Write request to WASM memory
        let req_ptr = self.wasm_write(&request_bytes)?;
        let req_len = request_bytes.len() as i32;

        // Allocate output area for response pointer and length (2 x i32 = 8 bytes)
        let out_area_ptr = self.alloc_raw(8)?;
        let resp_ptr_out = out_area_ptr;
        let resp_len_out = out_area_ptr + 4;

        let response = loop {
            // Call actr_handle
            let result = self
                .actr_handle
                .call(
                    &mut self.store,
                    (req_ptr, req_len, resp_ptr_out, resp_len_out),
                )
                .map_err(|e| WasmError::ExecutionFailed(format!("actr_handle call failed: {e}")))?;

            match self.store.data().asyncify_mode {
                AsyncifyMode::Unwinding => {
                    // WASM has saved state, stop unwind
                    self.asyncify_stop_unwind
                        .call(&mut self.store, ())
                        .map_err(|e| {
                            WasmError::ExecutionFailed(format!("asyncify_stop_unwind failed: {e}"))
                        })?;

                    // Take the pending IO request
                    let pending = self.store.data_mut().pending_call.take().ok_or_else(|| {
                        WasmError::ExecutionFailed("Unwinding but no pending_call".into())
                    })?;

                    tracing::debug!(call = ?std::mem::discriminant(&pending), "WASM initiated outbound call");

                    // Execute actual IO (async)
                    let io_result = call_executor(pending).await;

                    // Write result back to HostData, prepare for rewind
                    self.store.data_mut().io_result = Some(io_result);
                    self.store.data_mut().asyncify_mode = AsyncifyMode::Rewinding;

                    let data_ptr = self.store.data().asyncify_data_ptr;
                    self.asyncify_start_rewind
                        .call(&mut self.store, data_ptr)
                        .map_err(|e| {
                            WasmError::ExecutionFailed(format!("asyncify_start_rewind failed: {e}"))
                        })?;
                    // Continue loop, re-call actr_handle to trigger rewind
                }
                AsyncifyMode::Normal => {
                    // Normal completion (including completion after rewind)
                    if result != abi_code::SUCCESS {
                        self.store.data_mut().current_invocation = None;
                        self.free_raw(out_area_ptr, 8)?;
                        self.wasm_free(req_ptr, req_len)?;
                        return Err(WasmError::ExecutionFailed(format!(
                            "actr_handle returned error code {result} ({})",
                            abi::describe_error_code(result)
                        )));
                    }

                    let resp_ptr = self.read_i32(resp_ptr_out)?;
                    let resp_len = self.read_i32(resp_len_out)?;
                    self.free_raw(out_area_ptr, 8)?;
                    self.wasm_free(req_ptr, req_len)?;

                    if resp_ptr == 0 || resp_len <= 0 {
                        break Vec::new();
                    }

                    let data = self.wasm_read(resp_ptr, resp_len as usize)?;
                    self.wasm_free(resp_ptr, resp_len)?;

                    if self.legacy_handle_payload {
                        tracing::debug!(
                            req_bytes = request_bytes.len(),
                            resp_bytes = data.len(),
                            "legacy actr_handle completed"
                        );
                        break data;
                    }

                    let reply = guest_abi::decode_message::<AbiReply>(&data).map_err(|code| {
                        WasmError::ExecutionFailed(format!(
                            "guest returned malformed AbiReply with code {code}"
                        ))
                    })?;

                    if reply.status != guest_abi::code::SUCCESS {
                        self.store.data_mut().current_invocation = None;
                        let message = String::from_utf8(reply.payload)
                            .unwrap_or_else(|_| format!("guest returned status {}", reply.status));
                        return Err(WasmError::ExecutionFailed(message));
                    }

                    tracing::debug!(
                        req_bytes = request_bytes.len(),
                        resp_bytes = reply.payload.len(),
                        "actr_handle completed"
                    );

                    break reply.payload;
                }
                AsyncifyMode::Rewinding => {
                    // Should not be in Rewinding state when actr_handle returns
                    self.store.data_mut().current_invocation = None;
                    return Err(WasmError::ExecutionFailed(
                        "drive loop: actr_handle returned while still in Rewinding state".into(),
                    ));
                }
            }
        };

        self.store.data_mut().current_invocation = None;
        Ok(response)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Memory operation helper methods
// ─────────────────────────────────────────────────────────────────────────────

impl WasmWorkload {
    fn alloc_raw(&mut self, size: i32) -> WasmResult<i32> {
        let ptr = self
            .actr_alloc
            .call(&mut self.store, size)
            .map_err(|e| WasmError::ExecutionFailed(format!("actr_alloc failed: {e}")))?;
        if ptr == 0 {
            return Err(WasmError::ExecutionFailed(format!(
                "actr_alloc({size}) returned null (OOM)"
            )));
        }
        Ok(ptr)
    }

    fn free_raw(&mut self, ptr: i32, size: i32) -> WasmResult<()> {
        self.actr_free
            .call(&mut self.store, (ptr, size))
            .map_err(|e| WasmError::ExecutionFailed(format!("actr_free failed: {e}")))
    }

    fn wasm_write(&mut self, bytes: &[u8]) -> WasmResult<i32> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let ptr = self.alloc_raw(bytes.len() as i32)?;
        let mem = self.memory.data_mut(&mut self.store);
        let start = ptr as usize;
        let end = start + bytes.len();
        if end > mem.len() {
            return Err(WasmError::ExecutionFailed(format!(
                "write out of bounds: {start}..{end}, memory size {}",
                mem.len()
            )));
        }
        mem[start..end].copy_from_slice(bytes);
        Ok(ptr)
    }

    fn wasm_read(&mut self, ptr: i32, len: usize) -> WasmResult<Vec<u8>> {
        let mem = self.memory.data(&self.store);
        let start = ptr as usize;
        let end = start + len;
        if end > mem.len() {
            return Err(WasmError::ExecutionFailed(format!(
                "read out of bounds: {start}..{end}, memory size {}",
                mem.len()
            )));
        }
        Ok(mem[start..end].to_vec())
    }

    fn wasm_free(&mut self, ptr: i32, len: i32) -> WasmResult<()> {
        if ptr != 0 && len > 0 {
            self.free_raw(ptr, len)?;
        }
        Ok(())
    }

    fn read_i32(&mut self, ptr: i32) -> WasmResult<i32> {
        let bytes = self.wasm_read(ptr, 4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Write bytes directly to WASM linear memory at the specified address (no alloc, address provided by guest)
    pub fn write_to_addr(&mut self, ptr: i32, bytes: &[u8]) -> WasmResult<()> {
        let mem = self.memory.data_mut(&mut self.store);
        let start = ptr as usize;
        let end = start + bytes.len();
        if end > mem.len() {
            return Err(WasmError::ExecutionFailed(format!(
                "address write out of bounds: {start}..{end}"
            )));
        }
        mem[start..end].copy_from_slice(bytes);
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Asyncify initialization helpers
// ─────────────────────────────────────────────────────────────────────────────

fn init_asyncify_data(instance: &Instance, store: &mut Store<HostData>) {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .expect("no memory export");
    let mem = memory.data_mut(&mut *store);
    let base = ASYNCIFY_DATA_PTR as usize;
    mem[base..base + 4].copy_from_slice(&ASYNCIFY_STACK_START.to_le_bytes());
    mem[base + 4..base + 8].copy_from_slice(&ASYNCIFY_STACK_END.to_le_bytes());
    store.data_mut().asyncify_data_ptr = ASYNCIFY_DATA_PTR;
}

/// Reset the asyncify data buffer's write pointer before each dispatch
///
/// After unwind execution, the write pointer advances (local variable snapshot written).
/// Before the next dispatch, the write pointer must be reset to avoid snapshot area overflow
/// or overwriting old data.
fn reset_asyncify_data(store: &mut Store<HostData>, memory: &Memory) {
    let mem = memory.data_mut(&mut *store);
    let base = ASYNCIFY_DATA_PTR as usize;
    // Only reset the write pointer ([ptr+0]), stack end address ([ptr+4]) remains unchanged
    mem[base..base + 4].copy_from_slice(&ASYNCIFY_STACK_START.to_le_bytes());
}

// ─────────────────────────────────────────────────────────────────────────────
// Host import helper functions (called within Caller context)
// ─────────────────────────────────────────────────────────────────────────────

use crate::workload::decode_host_operation;

/// Read bytes from the specified range in WASM linear memory
fn read_bytes_from_wasm(caller: &mut Caller<HostData>, ptr: i32, len: i32) -> Vec<u8> {
    if ptr == 0 || len <= 0 {
        return Vec::new();
    }
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("no memory export");
    let data = mem.data(&*caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        tracing::warn!(
            start,
            end,
            mem_len = data.len(),
            "read_bytes_from_wasm out of bounds"
        );
        return Vec::new();
    }
    data[start..end].to_vec()
}

/// Write bytes to the specified address in WASM linear memory, return actual bytes written
fn write_to_wasm(caller: &mut Caller<HostData>, bytes: &[u8], ptr: i32, max: i32) -> i32 {
    if ptr == 0 || max <= 0 {
        return 0;
    }
    let to_write = bytes.len().min(max as usize);
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("no memory export");
    let data = mem.data_mut(caller);
    let start = ptr as usize;
    let end = start + to_write;
    if end > data.len() {
        tracing::warn!(start, end, "write_to_wasm out of bounds");
        return 0;
    }
    data[start..end].copy_from_slice(&bytes[..to_write]);
    to_write as i32
}

/// Write an i32 to WASM linear memory (little-endian)
fn write_i32_to_wasm(caller: &mut Caller<HostData>, ptr: i32, value: i32) {
    if ptr == 0 {
        return;
    }
    write_to_wasm(caller, &value.to_le_bytes(), ptr, 4);
}

fn write_legacy_context_bytes(
    caller: &mut Caller<HostData>,
    bytes: &[u8],
    buf_ptr: i32,
    buf_cap: i32,
) -> i32 {
    if buf_cap < bytes.len() as i32 {
        return bytes.len() as i32;
    }
    write_to_wasm(caller, bytes, buf_ptr, buf_cap)
}

/// Trigger asyncify unwind within a host import
fn trigger_unwind(caller: &mut Caller<HostData>) {
    let data_ptr = caller.data().asyncify_data_ptr;
    let start_unwind = caller
        .get_export("asyncify_start_unwind")
        .and_then(|e| e.into_func())
        .expect("asyncify_start_unwind not found");
    start_unwind
        .typed::<i32, ()>(&*caller)
        .expect("asyncify_start_unwind signature mismatch")
        .call(&mut *caller, data_ptr)
        .expect("asyncify_start_unwind call failed");
}

/// Stop asyncify rewind within a host import
fn trigger_stop_rewind(caller: &mut Caller<HostData>) {
    let stop_rewind = caller
        .get_export("asyncify_stop_rewind")
        .and_then(|e| e.into_func())
        .expect("asyncify_stop_rewind not found");
    stop_rewind
        .typed::<(), ()>(&*caller)
        .expect("asyncify_stop_rewind signature mismatch")
        .call(&mut *caller, ())
        .expect("asyncify_stop_rewind call failed");
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions
// ─────────────────────────────────────────────────────────────────────────────

fn resolve_func<Args, Ret>(
    instance: &Instance,
    store: &mut Store<HostData>,
    name: &str,
) -> WasmResult<TypedFunc<Args, Ret>>
where
    Args: wasmtime::WasmParams,
    Ret: wasmtime::WasmResults,
{
    instance
        .get_typed_func::<Args, Ret>(store, name)
        .map_err(|e| {
            WasmError::LoadFailed(format!(
                "export function '{name}' missing or signature mismatch: {e}"
            ))
        })
}

fn uses_legacy_handle_payload(module: &Module) -> bool {
    module.imports().any(|import| {
        import.module() == "env"
            && matches!(
                import.name(),
                "actr_host_self_id" | "actr_host_caller_id" | "actr_host_request_id"
            )
    })
}
