//! # actr-runtime-dynclib
//!
//! Actor-RTC cdylib guest 侧 runtime，运行在 native 共享库 (.so/.dylib/.dll) 中。
//!
//! 本 crate 是 `actr-runtime-wasm` 的 native 对应物：
//! - **`actr-runtime-wasm`**：WASM 客体侧，通过 host imports 与宿主通信
//! - **`actr-runtime-dynclib`**：native cdylib 客体侧，通过 HostVTable 函数指针与宿主通信
//!
//! ## 架构位置
//!
//! ```text
//! actor 业务代码（actr-framework 接口）
//!         ↓ 编译为 cdylib (.so/.dylib/.dll)
//! actr-runtime-dynclib（本 crate，编译进共享库）
//!         ↓ HostVTable 函数指针
//! actr-hyper（宿主侧，dlopen 加载并调用导出函数）
//! ```
//!
//! ## 使用方式
//!
//! ```rust,ignore
//! use actr_runtime_dynclib::entry_dynclib;
//!
//! // 1. 实现 Handler（通过 actr-framework 接口）
//! struct MyService;
//! // impl EchoServiceHandler for MyService { ... }
//!
//! // 2. 注册 Workload，生成 cdylib ABI 导出
//! entry_dynclib!(EchoServiceWorkload<MyService>);
//! ```
//!
//! ## 与 WASM guest 的区别
//!
//! - **共享地址空间**：SO 与宿主共享进程地址空间，无需 alloc/free 中转
//! - **VTable 回调**：通过函数指针表替代 WASM host imports
//! - **响应由 guest 分配**：`actr_handle` 在 guest 堆上分配响应，宿主通过 `actr_free_response` 释放

pub mod abi;
pub mod context;
pub mod vtable;

// 便捷重导出
pub use context::DynclibContext;
pub use vtable::HostVTable;

/// 为 Workload 类型生成 cdylib ABI 导出函数
///
/// # 参数
///
/// - `$workload_type`：实现了 `actr_framework::Workload + Send + Sync + 'static` 的类型
/// - `$init_expr`（可选）：构造 Workload 实例的表达式；省略时使用 `Default::default()`
///
/// # 生成的导出
///
/// - `actr_init(vtable, config_ptr, config_len) -> i32`
/// - `actr_handle(req_ptr, req_len, resp_out, resp_len_out) -> i32`
/// - `actr_free_response(ptr, len)`
#[macro_export]
macro_rules! entry_dynclib {
    // 单参数形式：使用 Default::default() 初始化
    ($workload_type:ty) => {
        $crate::entry_dynclib!($workload_type, <$workload_type as Default>::default());
    };

    // 双参数形式：使用自定义初始化表达式
    ($workload_type:ty, $init_expr:expr) => {
        static mut __ACTR_WORKLOAD: Option<$workload_type> = None;
        static mut __ACTR_VTABLE: Option<*const $crate::vtable::HostVTable> = None;

        /// 初始化 actor
        ///
        /// 宿主在 dlopen 后首次调用，传入 HostVTable 和可选的配置数据。
        /// 返回 0 表示成功，负值表示错误。
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

        /// 处理一条 RPC 请求
        ///
        /// - `req_ptr/req_len`：protobuf 编码的 `RpcEnvelope`
        /// - `resp_out`：指向 `*mut u8` 的指针，函数在此写入响应缓冲区地址
        /// - `resp_len_out`：指向 `usize` 的指针，函数在此写入响应数据长度
        ///
        /// 响应缓冲区由 guest 堆分配，宿主使用完毕后必须调用 `actr_free_response` 释放。
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn actr_handle(
            req_ptr: *const u8,
            req_len: usize,
            resp_out: *mut *mut u8,
            resp_len_out: *mut usize,
        ) -> i32 {
            use actr_protocol::prost::Message as ProstMessage;
            use actr_framework::{MessageDispatcher, Workload};

            // 获取 vtable
            let vtable = match unsafe { __ACTR_VTABLE } {
                Some(vt) => vt,
                None => return $crate::abi::code::INIT_FAILED,
            };

            // 读取请求 envelope
            if req_ptr.is_null() {
                return $crate::abi::code::PROTOCOL_ERROR;
            }
            let req_bytes = unsafe { std::slice::from_raw_parts(req_ptr, req_len) };

            let envelope = match actr_protocol::RpcEnvelope::decode(req_bytes) {
                Ok(e) => e,
                Err(_) => return $crate::abi::code::PROTOCOL_ERROR,
            };

            // 构建 DynclibContext（从 vtable 获取当前调用的上下文数据）
            let ctx = match unsafe { $crate::context::DynclibContext::from_vtable(vtable) } {
                Ok(c) => c,
                Err(_) => return $crate::abi::code::HANDLE_FAILED,
            };

            // 获取 workload 引用
            let workload = unsafe {
                match __ACTR_WORKLOAD.as_ref() {
                    Some(w) => w,
                    None => return $crate::abi::code::INIT_FAILED,
                }
            };

            // 通过 MessageDispatcher 路由并执行
            type Dispatcher = <$workload_type as Workload>::Dispatcher;

            // cdylib 是 native 环境，使用 tokio 或同步执行均可
            // 这里使用与 WASM 相同的单线程 poll 策略：
            // 所有 host 回调（vtable 函数指针）是同步的，Future 一次 poll 即完成。
            let resp_result = {
                let fut = Dispatcher::dispatch(workload, envelope, &ctx);
                // 构造 noop waker 同步驱动 future
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
                        return $crate::abi::code::HANDLE_FAILED;
                    }
                }
            };

            let resp_bytes = match resp_result {
                Ok(b) => b,
                Err(_) => return $crate::abi::code::HANDLE_FAILED,
            };

            // 在 guest 堆上分配响应缓冲区
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

        /// 释放 guest 分配的响应缓冲区
        ///
        /// 宿主在使用完 `actr_handle` 返回的响应数据后调用此函数释放内存。
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
