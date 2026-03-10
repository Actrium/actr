pub mod handle;
pub mod monitor;
pub mod spawn;

pub use handle::{ActrSystemHandle, ChildProcessHandle, ChildProcessState, WasmInstanceHandle};
pub use monitor::monitor_process;
pub use spawn::{RestartPolicy, SpawnConfig};

use std::sync::Arc;

/// Hyper 对所管理的 ActrSystem+Workload 栈的内部表示
///
/// 三种模式下 Hyper 的介入深度不同：
/// - Native：同进程，持有 ActrSystem 句柄
/// - Process：独立子进程，只做生命周期管理（不代理消息）
/// - Wasm：ActrSystem native shell 持有 WASM 引擎（WasmEngine 是 ActrSystem 内部 trait）
pub enum ActorRuntime {
    /// Mode 1 — Native
    ///
    /// ActrSystem+Workload 编译进同一 binary（或 FFI 静态链接），协程运行。
    /// Hyper 直接持有 ActrSystem 句柄，通过 `ActrSystemHandle` trait 管理生命周期。
    Native(Arc<dyn ActrSystemHandle>),

    /// Mode 2 — Process
    ///
    /// ActrSystem+Workload 作为独立 OS 进程运行。
    /// Hyper 完成签名验证 + AIS 注册后 spawn 子进程，credential 通过环境变量传递。
    /// 子进程内的 ActrSystem 直连 signaling，消息流量不经过 Hyper。
    /// Hyper 只做生命周期管理：health check、restart policy。
    Process(Box<ChildProcessHandle>),

    /// Mode 3 — WASM
    ///
    /// ActrSystem+Workload 整体编译为 .wasm，由 ActrSystem native shell 加载执行。
    /// WASM engine 是 ActrSystem 内部关注点，Hyper 不感知具体引擎实现。
    /// WASM 内的 ActrSystem 通过 Hyper 宿主函数访问外部能力（存储、加密、网络 I/O）。
    Wasm(WasmInstanceHandle),
}

impl ActorRuntime {
    /// ActrType 字符串（日志/调试用）
    pub fn actr_type(&self) -> &str {
        match self {
            ActorRuntime::Native(h) => h.id(),
            ActorRuntime::Process(h) => &h.actr_type,
            ActorRuntime::Wasm(h) => &h.actr_type,
        }
    }

    /// 是否健康
    pub fn is_healthy(&self) -> bool {
        match self {
            ActorRuntime::Native(h) => h.is_healthy(),
            ActorRuntime::Process(h) => h.is_running(),
            ActorRuntime::Wasm(_) => true, // WASM 健康状态由 ActrSystem shell 维护
        }
    }

    /// 运行模式名称（日志用）
    pub fn mode_name(&self) -> &'static str {
        match self {
            ActorRuntime::Native(_) => "native",
            ActorRuntime::Process(_) => "process",
            ActorRuntime::Wasm(_) => "wasm",
        }
    }
}
