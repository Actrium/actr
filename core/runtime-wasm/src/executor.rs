//! WASM 单线程 Future 执行器
//!
//! WASM 是单线程环境，actor 业务代码通过 asyncify 实现透明的 host import 挂起/恢复。
//! 从 Rust Future 的角度看，所有调用均同步完成（一次 poll 即返回 `Ready`）。
//!
//! # 工作原理
//!
//! 当 `WasmContext::call(...)` 调用 `actr_host_call` host import 时：
//! 1. **Normal 模式**：宿主调用 `asyncify_start_unwind`，WASM 保存整个调用栈（包括此执行器的栈帧）后返回
//! 2. **Rewinding 模式**：宿主调用 `asyncify_start_rewind` 后重新进入 WASM，从保存点恢复执行
//! 3. Host import 在 rewind 路径上返回实际 IO 结果，Future 继续执行直到 `Poll::Ready`
//!
//! 因此，`poll` 只会返回 `Poll::Ready`，`Poll::Pending` 表示程序错误。

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

/// 在 WASM 单线程环境中同步驱动 Future 到完成
///
/// # Panics
///
/// 如果 Future 返回 `Poll::Pending`（不应发生），panic 并报错。
/// 业务代码中的所有 `.await` 均通过 asyncify 透明处理，不会产生真正的 Pending。
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    match f.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => {
            // 这不应该发生：WASM + asyncify 保证所有 await 点同步完成
            panic!("Future 在 WASM 环境中返回 Pending，请确认所有 await 点均通过 asyncify 处理")
        }
    }
}

// ── Noop Waker ────────────────────────────────────────────────────────────────

/// 构造一个什么都不做的 Waker
///
/// 因为 WASM 执行器不需要 wake 通知——Future 始终在第一次 poll 时完成。
fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |p| RawWaker::new(p, &VTABLE), // clone
        |_| {},                         // wake
        |_| {},                         // wake_by_ref
        |_| {},                         // drop
    );
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

// ── 辅助：将 Pin<Box<dyn Future>> 同步执行 ───────────────────────────────────

/// 同步执行 `Pin<Box<dyn Future<Output = T> + Send>>`
///
/// 用于执行 `#[async_trait]` 生成的 boxed future。
pub fn block_on_boxed<T>(f: Pin<Box<dyn Future<Output = T> + Send>>) -> T {
    block_on(f)
}
