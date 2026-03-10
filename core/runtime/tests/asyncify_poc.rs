//! Asyncify 协议 POC 验证
//!
//! 验证目标：一个调用 host import 的 WASM 模块，经过 wasm-opt --asyncify 变换后，
//! host 能够在 host import 调用点挂起 WASM、执行异步 IO、再恢复 WASM 继续执行。
//!
//! 协议序列：
//! 1. Host 调用 `compute(x)`（WASM export）
//! 2. WASM 调用 `host_get_value(x)`（host import）
//! 3. Host import 内：调用 `asyncify_start_unwind(data_ptr)` → 返回 dummy 值
//! 4. WASM asyncify 代码检测到 unwind 状态 → 保存所有局部变量到 data buffer → 返回
//! 5. Host 调用 `asyncify_stop_unwind()`
//! 6. Host 执行"异步 IO"（此处 mock：返回 x * 2）
//! 7. Host 调用 `asyncify_start_rewind(data_ptr)`
//! 8. Host 重新调用 `compute(x)`
//! 9. WASM asyncify 代码检测到 rewind 状态 → 从 buffer 恢复状态 → 重新调用 `host_get_value`
//! 10. Host import 内：调用 `asyncify_stop_rewind()` → 返回真实 IO 结果
//! 11. WASM 继续正常执行，完成 `x + from_host`，返回结果

#![cfg(feature = "wasm-engine")]

// asyncify 变换后的 WASM fixture bytes（由 build.sh 生成）
include!("asyncify_fixture.rs");

use wasmtime::{Caller, Engine, Instance, Linker, Module, Store};

// ─── asyncify 协议状态 ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
enum AsyncifyMode {
    #[default]
    Normal,
    Unwinding,
    Rewinding,
}

#[derive(Debug, Default)]
struct HostData {
    mode: AsyncifyMode,
    /// asyncify data buffer 在 WASM 线性内存中的起始地址
    data_ptr: i32,
    /// host import 收到的输入（unwind 时保存，用于触发 IO）
    pending_input: Option<i32>,
    /// "异步 IO" 完成后的结果（rewind 时返回给 WASM）
    rewind_result: Option<i32>,
}

// ─── asyncify data buffer 布局 ────────────────────────────────────────────────
//
// asyncify_start_unwind(ptr) 期望 WASM 线性内存 ptr 处有：
//   [ptr+0] i32 = asyncify 栈当前写入位置（初始 = ptr + 8）
//   [ptr+4] i32 = asyncify 栈结束位置
//
// 我们在 WASM page 0 末尾预留 4KB 给 asyncify stack：

const ASYNCIFY_DATA_PTR: i32 = 0x8000; // 32KB offset（WASM page 0 内）
const ASYNCIFY_STACK_START: i32 = ASYNCIFY_DATA_PTR + 8;
const ASYNCIFY_STACK_END: i32 = ASYNCIFY_DATA_PTR + 0x1000; // +4KB

// ─── 辅助：在 WASM 线性内存中初始化 asyncify data buffer ────────────────────

fn init_asyncify_data(instance: &Instance, store: &mut Store<HostData>) {
    let memory = instance.get_memory(&mut *store, "memory").expect("no memory export");
    let data = memory.data_mut(&mut *store);
    let base = ASYNCIFY_DATA_PTR as usize;
    // [ptr+0]: 当前栈写入位置 = ASYNCIFY_STACK_START
    data[base..base + 4].copy_from_slice(&ASYNCIFY_STACK_START.to_le_bytes());
    // [ptr+4]: 栈结束位置 = ASYNCIFY_STACK_END
    data[base + 4..base + 8].copy_from_slice(&ASYNCIFY_STACK_END.to_le_bytes());

    store.data_mut().data_ptr = ASYNCIFY_DATA_PTR;
}

// ─── asyncify 驱动循环 ────────────────────────────────────────────────────────

fn drive(instance: &Instance, store: &mut Store<HostData>, x: i32) -> i32 {
    let compute = instance
        .get_typed_func::<i32, i32>(&mut *store, "compute")
        .expect("no compute export");
    let stop_unwind = instance
        .get_typed_func::<(), ()>(&mut *store, "asyncify_stop_unwind")
        .expect("no asyncify_stop_unwind");
    let start_rewind = instance
        .get_typed_func::<i32, ()>(&mut *store, "asyncify_start_rewind")
        .expect("no asyncify_start_rewind");

    loop {
        let result = compute.call(&mut *store, x).expect("compute call failed");

        match store.data().mode {
            AsyncifyMode::Unwinding => {
                // WASM 已展开并保存状态，停止 unwind
                stop_unwind.call(&mut *store, ()).expect("stop_unwind failed");

                // 模拟"异步 IO"：返回 x * 2
                let input = store.data().pending_input.unwrap();
                let io_result = input * 2;
                tracing::debug!(input, io_result, "mock IO 完成");

                // 准备 rewind
                store.data_mut().rewind_result = Some(io_result);
                store.data_mut().mode = AsyncifyMode::Rewinding;

                let data_ptr = store.data().data_ptr;
                start_rewind.call(&mut *store, data_ptr).expect("start_rewind failed");
                // 注意：不能重置 data buffer！unwind 已将状态写入 buffer，
                // rewind 需要从 buffer 中读取恢复——重置会破坏已保存的状态。
                // 继续循环，重新调用 compute 触发 rewind
            }
            AsyncifyMode::Normal => {
                // 正常完成（含 rewind 后正常执行完毕的情况）
                return result;
            }
            AsyncifyMode::Rewinding => {
                // rewind 完成后 host import 会将 mode 设回 Normal，
                // 此处不应出现
                unreachable!("drive loop: unexpected Rewinding state after compute returned");
            }
        }
    }
}

// ─── 测试用例 ─────────────────────────────────────────────────────────────────

#[test]
fn asyncify_suspend_resume_basic() {
    let engine = Engine::default();
    let module = Module::new(&engine, ASYNCIFY_FIXTURE_WASM).expect("module load failed");

    let mut linker = Linker::<HostData>::new(&engine);

    // 注册 host import：host_get_value(x) -> i32
    // 在 Normal 模式下触发 unwind，在 Rewinding 模式下返回真实结果
    linker
        .func_wrap("env", "host_get_value", |mut caller: Caller<HostData>, x: i32| -> i32 {
            let mode = caller.data().mode.clone();
            match mode {
                AsyncifyMode::Rewinding => {
                    // Rewind 阶段：返回之前 IO 计算好的结果，停止 rewind
                    let result = caller.data_mut().rewind_result.take().unwrap();
                    caller.data_mut().mode = AsyncifyMode::Normal;

                    // 调用 asyncify_stop_rewind()
                    let stop_rewind = caller
                        .get_export("asyncify_stop_rewind")
                        .and_then(|e| e.into_func())
                        .expect("no asyncify_stop_rewind");
                    stop_rewind
                        .typed::<(), ()>(&caller)
                        .unwrap()
                        .call(&mut caller, ())
                        .expect("stop_rewind failed");

                    tracing::debug!(result, "host_get_value rewind → 返回真实结果");
                    result
                }
                AsyncifyMode::Normal => {
                    // 首次调用：触发 unwind，挂起 WASM
                    caller.data_mut().pending_input = Some(x);
                    caller.data_mut().mode = AsyncifyMode::Unwinding;

                    let data_ptr = caller.data().data_ptr;

                    // 调用 asyncify_start_unwind(data_ptr)
                    let start_unwind = caller
                        .get_export("asyncify_start_unwind")
                        .and_then(|e| e.into_func())
                        .expect("no asyncify_start_unwind");
                    start_unwind
                        .typed::<i32, ()>(&caller)
                        .unwrap()
                        .call(&mut caller, data_ptr)
                        .expect("start_unwind failed");

                    tracing::debug!(x, "host_get_value 触发 unwind，挂起 WASM");
                    0 // dummy，unwind 期间会被忽略
                }
                AsyncifyMode::Unwinding => {
                    // 不应在 unwind 期间再次调用 host import
                    unreachable!("host_get_value called while unwinding")
                }
            }
        })
        .expect("linker func_wrap failed");

    let mut store = Store::new(&engine, HostData::default());
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("instantiate failed");

    init_asyncify_data(&instance, &mut store);

    // compute(5)：WASM 调用 host_get_value(5)，host mock 返回 10（5*2）
    // 预期：5 + 10 = 15
    let result = drive(&instance, &mut store, 5);
    assert_eq!(result, 15, "asyncify suspend/resume 结果应为 15");

    tracing::info!(result, "✅ asyncify POC 验证通过");
}

#[test]
fn asyncify_multiple_calls() {
    let engine = Engine::default();
    let module = Module::new(&engine, ASYNCIFY_FIXTURE_WASM).expect("module load failed");

    let mut linker = Linker::<HostData>::new(&engine);
    linker
        .func_wrap("env", "host_get_value", |mut caller: Caller<HostData>, x: i32| -> i32 {
            let mode = caller.data().mode.clone();
            match mode {
                AsyncifyMode::Rewinding => {
                    let result = caller.data_mut().rewind_result.take().unwrap();
                    caller.data_mut().mode = AsyncifyMode::Normal;
                    let stop_rewind = caller
                        .get_export("asyncify_stop_rewind")
                        .and_then(|e| e.into_func())
                        .unwrap();
                    stop_rewind
                        .typed::<(), ()>(&caller)
                        .unwrap()
                        .call(&mut caller, ())
                        .unwrap();
                    result
                }
                AsyncifyMode::Normal => {
                    caller.data_mut().pending_input = Some(x);
                    caller.data_mut().mode = AsyncifyMode::Unwinding;
                    let data_ptr = caller.data().data_ptr;
                    let start_unwind = caller
                        .get_export("asyncify_start_unwind")
                        .and_then(|e| e.into_func())
                        .unwrap();
                    start_unwind
                        .typed::<i32, ()>(&caller)
                        .unwrap()
                        .call(&mut caller, data_ptr)
                        .unwrap();
                    0
                }
                AsyncifyMode::Unwinding => unreachable!(),
            }
        })
        .unwrap();

    // 多次调用，每次独立的 instance（asyncify data buffer 重置）
    for x in [1i32, 7, 42, 100] {
        let mut store = Store::new(&engine, HostData::default());
        let instance = linker.instantiate(&mut store, &module).unwrap();
        init_asyncify_data(&instance, &mut store);

        let result = drive(&instance, &mut store, x);
        assert_eq!(result, x + x * 2, "compute({x}) 应返回 {}", x + x * 2);
    }

    tracing::info!("✅ asyncify 多次调用验证通过");
}
