//! WasmHost — Wasmtime 宿主引擎
//!
//! 实现了从 WASM 字节加载、编译、实例化到消息分发的完整生命周期。
//! 单个 `WasmHost` 对应一个 WASM 模块（编译一次），可派生多个 `WasmInstance`。
//!
//! # Asyncify 驱动
//!
//! `dispatch()` 使用 asyncify unwind/rewind 协议：
//! 1. 调用 `actr_handle`
//! 2. 若 WASM 触发 host import（如 `actr_host_call`），import 内发起 unwind 并挂起
//! 3. drive 循环检测到 Unwinding，执行真实异步 IO
//! 4. 将结果存入 `HostData`，发起 rewind，重新调用 `actr_handle`
//! 5. Import 在 Rewinding 模式下返回真实结果，WASM 继续执行

use actr_protocol::{ActrId, prost::Message as ProstMessage};
use wasmtime::{Caller, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::wasm::error::{WasmError, WasmResult};

use super::abi::{self, WasmActorConfig};

// ─────────────────────────────────────────────────────────────────────────────
// HostData — Store 内保存的运行时状态
// ─────────────────────────────────────────────────────────────────────────────

/// Asyncify 状态机
#[derive(Debug, Clone, PartialEq, Default)]
enum AsyncifyMode {
    #[default]
    Normal,
    Unwinding,
    Rewinding,
}

/// 当前调用的上下文数据（由 dispatch 调用方设置，host import 读取）
#[derive(Debug, Default)]
pub struct DispatchContext {
    pub self_id: ActrId,
    pub caller_id: Option<ActrId>,
    pub request_id: String,
}

/// WASM guest 发起的出站调用请求
///
/// out_ptr / out_len_ptr 等输出地址字段不在此处保存——rewind 时 host import
/// 会被再次调用，参数与 unwind 时完全相同，因此直接从参数读取即可。
#[derive(Debug)]
pub enum PendingCall {
    /// 有响应的 RPC 调用（经 Dest 路由）
    Call {
        route_key: String,
        dest_bytes: Vec<u8>,
        payload: Vec<u8>,
    },
    /// 无响应的单向消息
    Tell {
        route_key: String,
        dest_bytes: Vec<u8>,
        payload: Vec<u8>,
    },
    /// 服务发现（按 ActrType）
    Discover { type_bytes: Vec<u8> },
    /// 原始 RPC 调用（按 ActrId 直接路由）
    CallRaw {
        route_key: String,
        target_bytes: Vec<u8>,
        payload: Vec<u8>,
    },
}

/// `call_executor` 返回给 drive 循环的 IO 结果
///
/// host import 在 Rewinding 模式下读取此值并写入 WASM 线性内存。
#[derive(Debug)]
pub enum IoResult {
    /// Call / CallRaw / Discover 的响应字节（由 host import 在 rewind 时写入 WASM 内存）
    Bytes(Vec<u8>),
    /// Tell 完成（无响应数据）
    Done,
    /// 错误码
    Error(i32),
}

/// Wasmtime Store 内部数据
#[derive(Debug, Default)]
struct HostData {
    // ── asyncify 协议 ──────────────────────────────────────────────────────
    asyncify_mode: AsyncifyMode,
    asyncify_data_ptr: i32,
    // ── 当前请求上下文（dispatch 开始时设置）────────────────────────────────
    ctx: DispatchContext,
    // ── host import 挂起时保存的待执行 IO ───────────────────────────────────
    pending_call: Option<PendingCall>,
    // ── drive 循环将 IO 结果写回此处，rewind 时 host import 读取 ───────────
    io_result: Option<IoResult>,
}

// asyncify data buffer 布局（固定地址，WASM page 0 内）
const ASYNCIFY_DATA_PTR: i32 = 0x8000; // 32 KB
const ASYNCIFY_STACK_START: i32 = ASYNCIFY_DATA_PTR + 8;
const ASYNCIFY_STACK_END: i32 = ASYNCIFY_DATA_PTR + 0x1000; // +4 KB

// ─────────────────────────────────────────────────────────────────────────────
// WasmHost
// ─────────────────────────────────────────────────────────────────────────────

/// WASM 宿主引擎
///
/// 编译并持有 WASM 模块，同一模块可多次实例化（每个 actor 一个实例）。
/// 编译是 CPU 密集型操作，应只执行一次后复用 `WasmHost`。
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
    /// 从 WASM 字节编译模块（CPU 密集，建议在 `spawn_blocking` 中调用）
    pub fn compile(wasm_bytes: &[u8]) -> WasmResult<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| WasmError::LoadFailed(format!("模块编译失败: {e}")))?;

        tracing::info!(wasm_bytes = wasm_bytes.len(), "WASM 模块编译完成");
        Ok(Self { engine, module })
    }

    /// 实例化 WASM 模块，注册所有 host imports，返回可执行的 `WasmInstance`
    pub fn instantiate(&self) -> WasmResult<WasmInstance> {
        let mut linker = Linker::<HostData>::new(&self.engine);
        register_host_imports(&mut linker)?;

        let mut store = Store::new(&self.engine, HostData::default());

        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| WasmError::LoadFailed(format!("模块实例化失败: {e}")))?;

        // 初始化 asyncify data buffer
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
                    "WASM 模块未导出线性内存 'memory'".to_string(),
                )
            })?;

        // asyncify 控制函数（由 wasm-opt --asyncify 注入到 WASM 二进制）
        let asyncify_stop_unwind =
            resolve_func::<(), ()>(&instance, &mut store, "asyncify_stop_unwind")?;
        let asyncify_start_rewind =
            resolve_func::<i32, ()>(&instance, &mut store, "asyncify_start_rewind")?;

        tracing::info!("WASM 实例化成功，所有 ABI 导出函数验证通过");

        Ok(WasmInstance {
            store,
            _instance: instance,
            actr_init,
            actr_handle,
            actr_alloc,
            actr_free,
            memory,
            asyncify_stop_unwind,
            asyncify_start_rewind,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Host imports 注册
// ─────────────────────────────────────────────────────────────────────────────

fn register_host_imports(linker: &mut Linker<HostData>) -> WasmResult<()> {
    // ── 同步上下文访问器 ─────────────────────────────────────────────────────

    linker
        .func_wrap(
            "env",
            "actr_host_self_id",
            |mut caller: Caller<HostData>, out_ptr: i32, out_max: i32| -> i32 {
                let bytes = caller.data().ctx.self_id.encode_to_vec();
                write_to_wasm(&mut caller, &bytes, out_ptr, out_max)
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("注册 actr_host_self_id 失败: {e}")))?;

    linker
        .func_wrap(
            "env",
            "actr_host_caller_id",
            |mut caller: Caller<HostData>, out_ptr: i32, out_max: i32| -> i32 {
                match &caller.data().ctx.caller_id {
                    None => -1, // 无调用方
                    Some(id) => {
                        let bytes = id.encode_to_vec();
                        write_to_wasm(&mut caller, &bytes, out_ptr, out_max)
                    }
                }
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("注册 actr_host_caller_id 失败: {e}")))?;

    linker
        .func_wrap(
            "env",
            "actr_host_request_id",
            |mut caller: Caller<HostData>, out_ptr: i32, out_max: i32| -> i32 {
                let bytes = caller.data().ctx.request_id.as_bytes().to_vec();
                write_to_wasm(&mut caller, &bytes, out_ptr, out_max)
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("注册 actr_host_request_id 失败: {e}")))?;

    // ── 异步通信（asyncify 驱动）─────────────────────────────────────────────

    linker
        .func_wrap(
            "env",
            "actr_host_call",
            |mut caller: Caller<HostData>,
             route_key_ptr: i32,
             route_key_len: i32,
             dest_ptr: i32,
             dest_len: i32,
             payload_ptr: i32,
             payload_len: i32,
             out_ptr: i32,
             out_max: i32,
             out_len_ptr: i32|
             -> i32 {
                let mode = caller.data().asyncify_mode.clone();
                match mode {
                    AsyncifyMode::Normal => {
                        let route_key =
                            read_string_from_wasm(&mut caller, route_key_ptr, route_key_len);
                        let dest_bytes = read_bytes_from_wasm(&mut caller, dest_ptr, dest_len);
                        let payload = read_bytes_from_wasm(&mut caller, payload_ptr, payload_len);
                        caller.data_mut().pending_call =
                            Some(PendingCall::Call { route_key, dest_bytes, payload });
                        caller.data_mut().asyncify_mode = AsyncifyMode::Unwinding;
                        trigger_unwind(&mut caller);
                        0
                    }
                    AsyncifyMode::Rewinding => {
                        // rewind：从 io_result 取响应 bytes，写入 WASM 内存，设置长度
                        let code = match caller.data_mut().io_result.take() {
                            Some(IoResult::Bytes(bytes)) => {
                                let written =
                                    write_to_wasm(&mut caller, &bytes, out_ptr, out_max);
                                write_i32_to_wasm(&mut caller, out_len_ptr, written);
                                abi::code::SUCCESS
                            }
                            Some(IoResult::Error(c)) => c,
                            _ => abi::code::GENERIC_ERROR,
                        };
                        caller.data_mut().asyncify_mode = AsyncifyMode::Normal;
                        trigger_stop_rewind(&mut caller);
                        code
                    }
                    AsyncifyMode::Unwinding => 0,
                }
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("注册 actr_host_call 失败: {e}")))?;

    linker
        .func_wrap(
            "env",
            "actr_host_tell",
            |mut caller: Caller<HostData>,
             route_key_ptr: i32,
             route_key_len: i32,
             dest_ptr: i32,
             dest_len: i32,
             payload_ptr: i32,
             payload_len: i32|
             -> i32 {
                let mode = caller.data().asyncify_mode.clone();
                match mode {
                    AsyncifyMode::Normal => {
                        let route_key =
                            read_string_from_wasm(&mut caller, route_key_ptr, route_key_len);
                        let dest_bytes = read_bytes_from_wasm(&mut caller, dest_ptr, dest_len);
                        let payload = read_bytes_from_wasm(&mut caller, payload_ptr, payload_len);
                        caller.data_mut().pending_call =
                            Some(PendingCall::Tell { route_key, dest_bytes, payload });
                        caller.data_mut().asyncify_mode = AsyncifyMode::Unwinding;
                        trigger_unwind(&mut caller);
                        0
                    }
                    AsyncifyMode::Rewinding => {
                        let code = match caller.data_mut().io_result.take() {
                            Some(IoResult::Done) => abi::code::SUCCESS,
                            Some(IoResult::Error(c)) => c,
                            _ => abi::code::GENERIC_ERROR,
                        };
                        caller.data_mut().asyncify_mode = AsyncifyMode::Normal;
                        trigger_stop_rewind(&mut caller);
                        code
                    }
                    AsyncifyMode::Unwinding => 0,
                }
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("注册 actr_host_tell 失败: {e}")))?;

    linker
        .func_wrap(
            "env",
            "actr_host_call_raw",
            |mut caller: Caller<HostData>,
             route_key_ptr: i32,
             route_key_len: i32,
             target_ptr: i32,
             target_len: i32,
             payload_ptr: i32,
             payload_len: i32,
             out_ptr: i32,
             out_max: i32,
             out_len_ptr: i32|
             -> i32 {
                let mode = caller.data().asyncify_mode.clone();
                match mode {
                    AsyncifyMode::Normal => {
                        let route_key =
                            read_string_from_wasm(&mut caller, route_key_ptr, route_key_len);
                        let target_bytes =
                            read_bytes_from_wasm(&mut caller, target_ptr, target_len);
                        let payload = read_bytes_from_wasm(&mut caller, payload_ptr, payload_len);
                        caller.data_mut().pending_call =
                            Some(PendingCall::CallRaw { route_key, target_bytes, payload });
                        caller.data_mut().asyncify_mode = AsyncifyMode::Unwinding;
                        trigger_unwind(&mut caller);
                        0
                    }
                    AsyncifyMode::Rewinding => {
                        let code = match caller.data_mut().io_result.take() {
                            Some(IoResult::Bytes(bytes)) => {
                                let written =
                                    write_to_wasm(&mut caller, &bytes, out_ptr, out_max);
                                write_i32_to_wasm(&mut caller, out_len_ptr, written);
                                abi::code::SUCCESS
                            }
                            Some(IoResult::Error(c)) => c,
                            _ => abi::code::GENERIC_ERROR,
                        };
                        caller.data_mut().asyncify_mode = AsyncifyMode::Normal;
                        trigger_stop_rewind(&mut caller);
                        code
                    }
                    AsyncifyMode::Unwinding => 0,
                }
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("注册 actr_host_call_raw 失败: {e}")))?;

    linker
        .func_wrap(
            "env",
            "actr_host_discover",
            |mut caller: Caller<HostData>,
             type_ptr: i32,
             type_len: i32,
             out_ptr: i32,
             out_max: i32|
             -> i32 {
                let mode = caller.data().asyncify_mode.clone();
                match mode {
                    AsyncifyMode::Normal => {
                        let type_bytes = read_bytes_from_wasm(&mut caller, type_ptr, type_len);
                        caller.data_mut().pending_call =
                            Some(PendingCall::Discover { type_bytes });
                        caller.data_mut().asyncify_mode = AsyncifyMode::Unwinding;
                        trigger_unwind(&mut caller);
                        0
                    }
                    AsyncifyMode::Rewinding => {
                        // 返回实际写入字节数（discover 的返回值即为长度）
                        let code = match caller.data_mut().io_result.take() {
                            Some(IoResult::Bytes(bytes)) => {
                                write_to_wasm(&mut caller, &bytes, out_ptr, out_max)
                            }
                            Some(IoResult::Error(c)) => c,
                            _ => abi::code::GENERIC_ERROR,
                        };
                        caller.data_mut().asyncify_mode = AsyncifyMode::Normal;
                        trigger_stop_rewind(&mut caller);
                        code
                    }
                    AsyncifyMode::Unwinding => 0,
                }
            },
        )
        .map_err(|e| WasmError::LoadFailed(format!("注册 actr_host_discover 失败: {e}")))?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// WasmInstance
// ─────────────────────────────────────────────────────────────────────────────

/// 单个 WASM actor 实例
///
/// 封装 Wasmtime `Store<HostData>` 和已缓存的导出函数句柄。
/// **非 `Sync`**：调用方负责并发保护（通常是 `Mutex<WasmInstance>`）。
pub struct WasmInstance {
    store: Store<HostData>,
    _instance: Instance,
    actr_init: TypedFunc<(i32, i32), i32>,
    actr_handle: TypedFunc<(i32, i32, i32, i32), i32>,
    actr_alloc: TypedFunc<i32, i32>,
    actr_free: TypedFunc<(i32, i32), ()>,
    memory: Memory,
    asyncify_stop_unwind: TypedFunc<(), ()>,
    asyncify_start_rewind: TypedFunc<i32, ()>,
}

impl std::fmt::Debug for WasmInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmInstance").finish_non_exhaustive()
    }
}

impl WasmInstance {
    /// 初始化 WASM actor（调用 `actr_init`）
    pub fn init(&mut self, config: &WasmActorConfig) -> WasmResult<()> {
        let config_json = serde_json::to_vec(config)
            .map_err(|e| WasmError::InitFailed(format!("config 序列化失败: {e}")))?;

        let ptr = self.wasm_write(&config_json)?;
        let len = config_json.len() as i32;

        let result = self
            .actr_init
            .call(&mut self.store, (ptr, len))
            .map_err(|e| WasmError::InitFailed(format!("actr_init 调用失败: {e}")))?;

        self.wasm_free(ptr, len)?;

        if result != abi::code::SUCCESS {
            return Err(WasmError::InitFailed(format!(
                "actr_init 返回错误码 {result} ({})",
                abi::describe_error_code(result)
            )));
        }

        tracing::info!("WASM actor 初始化成功");
        Ok(())
    }

    /// 分发一条 RPC 请求，使用 asyncify 驱动循环处理出站 IO
    ///
    /// `ctx`：本次调用的上下文数据（self_id、caller_id、request_id）
    /// `call_executor`：处理 guest 发起的出站调用（返回 `IoResult`）
    ///
    /// 注意：`call_executor` 需要执行实际的异步 IO，但 Wasmtime 的 host import 是同步的。
    /// asyncify 协议将 "需要 IO" 的时机暴露到 drive 循环（此函数），
    /// 在循环内部可以调用 `call_executor`（async）。
    pub async fn dispatch<F, Fut>(
        &mut self,
        request_bytes: &[u8],
        ctx: DispatchContext,
        call_executor: F,
    ) -> WasmResult<Vec<u8>>
    where
        F: Fn(PendingCall) -> Fut,
        Fut: std::future::Future<Output = IoResult>,
    {
        // 每次 dispatch 前重置 asyncify data buffer
        // （前次 unwind 可能已写入 buffer，必须在新的 handle 调用前 reset 写指针）
        reset_asyncify_data(&mut self.store, &self.memory);

        // 设置本次调用的上下文
        self.store.data_mut().ctx = ctx;
        self.store.data_mut().asyncify_mode = AsyncifyMode::Normal;

        // 将请求写入 WASM 内存
        let req_ptr = self.wasm_write(request_bytes)?;
        let req_len = request_bytes.len() as i32;

        // 分配接收响应指针和长度的输出区域（2 × i32 = 8 字节）
        let out_area_ptr = self.alloc_raw(8)?;
        let resp_ptr_out = out_area_ptr;
        let resp_len_out = out_area_ptr + 4;

        let response = loop {
            // 调用 actr_handle
            let result = self
                .actr_handle
                .call(
                    &mut self.store,
                    (req_ptr, req_len, resp_ptr_out, resp_len_out),
                )
                .map_err(|e| {
                    WasmError::ExecutionFailed(format!("actr_handle 调用失败: {e}"))
                })?;

            match self.store.data().asyncify_mode {
                AsyncifyMode::Unwinding => {
                    // WASM 已保存状态，停止 unwind
                    self.asyncify_stop_unwind
                        .call(&mut self.store, ())
                        .map_err(|e| {
                            WasmError::ExecutionFailed(format!("asyncify_stop_unwind 失败: {e}"))
                        })?;

                    // 取出待执行的 IO 请求
                    let pending = self.store.data_mut().pending_call.take().ok_or_else(|| {
                        WasmError::ExecutionFailed("Unwinding 但无 pending_call".into())
                    })?;

                    tracing::debug!(call = ?std::mem::discriminant(&pending), "WASM 发起出站调用");

                    // 执行实际 IO（异步）
                    let io_result = call_executor(pending).await;

                    // 将结果写回 HostData，准备 rewind
                    self.store.data_mut().io_result = Some(io_result);
                    self.store.data_mut().asyncify_mode = AsyncifyMode::Rewinding;

                    let data_ptr = self.store.data().asyncify_data_ptr;
                    self.asyncify_start_rewind
                        .call(&mut self.store, data_ptr)
                        .map_err(|e| {
                            WasmError::ExecutionFailed(format!("asyncify_start_rewind 失败: {e}"))
                        })?;
                    // 继续循环，重新调用 actr_handle 触发 rewind
                }
                AsyncifyMode::Normal => {
                    // 正常完成（含 rewind 后执行完毕）
                    if result != abi::code::SUCCESS {
                        self.free_raw(out_area_ptr, 8)?;
                        self.wasm_free(req_ptr, req_len)?;
                        return Err(WasmError::ExecutionFailed(format!(
                            "actr_handle 返回错误码 {result} ({})",
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

                    tracing::debug!(
                        req_bytes = request_bytes.len(),
                        resp_bytes = data.len(),
                        "actr_handle 完成"
                    );

                    break data;
                }
                AsyncifyMode::Rewinding => {
                    // 不应在 actr_handle 返回时处于 Rewinding 状态
                    return Err(WasmError::ExecutionFailed(
                        "drive 循环：actr_handle 返回时仍处于 Rewinding 状态".into(),
                    ));
                }
            }
        };

        Ok(response)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 内存操作辅助方法
// ─────────────────────────────────────────────────────────────────────────────

impl WasmInstance {
    fn alloc_raw(&mut self, size: i32) -> WasmResult<i32> {
        let ptr = self
            .actr_alloc
            .call(&mut self.store, size)
            .map_err(|e| WasmError::ExecutionFailed(format!("actr_alloc 失败: {e}")))?;
        if ptr == 0 {
            return Err(WasmError::ExecutionFailed(format!(
                "actr_alloc({size}) 返回 null（OOM）"
            )));
        }
        Ok(ptr)
    }

    fn free_raw(&mut self, ptr: i32, size: i32) -> WasmResult<()> {
        self.actr_free
            .call(&mut self.store, (ptr, size))
            .map_err(|e| WasmError::ExecutionFailed(format!("actr_free 失败: {e}")))
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
                "写入越界：{start}..{end}，内存大小 {}",
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
                "读取越界：{start}..{end}，内存大小 {}",
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

    /// 将 bytes 直接写入 WASM 线性内存指定地址（不 alloc，地址由 guest 提供）
    pub fn write_to_addr(&mut self, ptr: i32, bytes: &[u8]) -> WasmResult<()> {
        let mem = self.memory.data_mut(&mut self.store);
        let start = ptr as usize;
        let end = start + bytes.len();
        if end > mem.len() {
            return Err(WasmError::ExecutionFailed(format!(
                "地址写入越界：{start}..{end}"
            )));
        }
        mem[start..end].copy_from_slice(bytes);
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Asyncify 初始化辅助
// ─────────────────────────────────────────────────────────────────────────────

fn init_asyncify_data(instance: &Instance, store: &mut Store<HostData>) {
    let memory = instance.get_memory(&mut *store, "memory").expect("no memory export");
    let mem = memory.data_mut(&mut *store);
    let base = ASYNCIFY_DATA_PTR as usize;
    mem[base..base + 4].copy_from_slice(&ASYNCIFY_STACK_START.to_le_bytes());
    mem[base + 4..base + 8].copy_from_slice(&ASYNCIFY_STACK_END.to_le_bytes());
    store.data_mut().asyncify_data_ptr = ASYNCIFY_DATA_PTR;
}

/// 每次 dispatch 前重置 asyncify data buffer 的写指针
///
/// unwind 执行后写指针会前移（已写入局部变量快照）。
/// 下一次 dispatch 开始前需要将写指针复位，否则快照区域溢出或覆盖旧数据。
fn reset_asyncify_data(store: &mut Store<HostData>, memory: &Memory) {
    let mem = memory.data_mut(&mut *store);
    let base = ASYNCIFY_DATA_PTR as usize;
    // 只重置写指针（[ptr+0]），栈结束地址（[ptr+4]）保持不变
    mem[base..base + 4].copy_from_slice(&ASYNCIFY_STACK_START.to_le_bytes());
}

// ─────────────────────────────────────────────────────────────────────────────
// Host import 辅助函数（在 Caller 上下文中调用）
// ─────────────────────────────────────────────────────────────────────────────

/// 从 WASM 线性内存读取指定范围的字节
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
        tracing::warn!(start, end, mem_len = data.len(), "read_bytes_from_wasm 越界");
        return Vec::new();
    }
    data[start..end].to_vec()
}

/// 从 WASM 线性内存读取 UTF-8 字符串
fn read_string_from_wasm(caller: &mut Caller<HostData>, ptr: i32, len: i32) -> String {
    let bytes = read_bytes_from_wasm(caller, ptr, len);
    String::from_utf8(bytes).unwrap_or_default()
}

/// 将 bytes 写入 WASM 线性内存指定地址，返回实际写入字节数
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
        tracing::warn!(start, end, "write_to_wasm 越界");
        return 0;
    }
    data[start..end].copy_from_slice(&bytes[..to_write]);
    to_write as i32
}

/// 将 i32 写入 WASM 线性内存（little-endian）
fn write_i32_to_wasm(caller: &mut Caller<HostData>, ptr: i32, value: i32) {
    if ptr == 0 {
        return;
    }
    write_to_wasm(caller, &value.to_le_bytes(), ptr, 4);
}

/// 在 host import 内触发 asyncify unwind
fn trigger_unwind(caller: &mut Caller<HostData>) {
    let data_ptr = caller.data().asyncify_data_ptr;
    let start_unwind = caller
        .get_export("asyncify_start_unwind")
        .and_then(|e| e.into_func())
        .expect("asyncify_start_unwind 未找到");
    start_unwind
        .typed::<i32, ()>(&*caller)
        .expect("asyncify_start_unwind 签名错误")
        .call(&mut *caller, data_ptr)
        .expect("asyncify_start_unwind 调用失败");
}

/// 在 host import 内停止 asyncify rewind
fn trigger_stop_rewind(caller: &mut Caller<HostData>) {
    let stop_rewind = caller
        .get_export("asyncify_stop_rewind")
        .and_then(|e| e.into_func())
        .expect("asyncify_stop_rewind 未找到");
    stop_rewind
        .typed::<(), ()>(&*caller)
        .expect("asyncify_stop_rewind 签名错误")
        .call(&mut *caller, ())
        .expect("asyncify_stop_rewind 调用失败");
}

// ─────────────────────────────────────────────────────────────────────────────
// 辅助函数
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
        .map_err(|e| WasmError::LoadFailed(format!("导出函数 '{name}' 缺失或签名不匹配: {e}")))
}
