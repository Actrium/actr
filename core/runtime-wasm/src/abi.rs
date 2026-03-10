//! WASM Guest-Host ABI 定义与 `entry!` 宏
//!
//! 本模块定义：
//! 1. **ABI 错误码**：与 `actr-runtime` 中的 `wasm/abi.rs` 保持一致
//! 2. **`entry!` 宏**：为 Workload 类型生成全部必要的 WASM ABI 导出函数
//!
//! # ABI 导出函数（由 `entry!` 宏生成）
//!
//! ```text
//! // 分配 WASM 线性内存（宿主在写入数据前调用）
//! actr_alloc(size: i32) -> i32
//!
//! // 释放 WASM 线性内存（宿主读写完毕后调用）
//! actr_free(ptr: i32, size: i32)
//!
//! // 初始化 actor（宿主在首次调用 actr_handle 前调用）
//! // config_ptr/len：JSON 编码的 WasmActorConfig
//! // 返回：0 成功，-2 失败
//! actr_init(config_ptr: i32, config_len: i32) -> i32
//!
//! // 处理一条 RPC 请求
//! // req_ptr/len：protobuf 编码的 RpcEnvelope
//! // resp_ptr_out：宿主提供的 i32* 指针，WASM 在此写入响应缓冲区地址
//! // resp_len_out：宿主提供的 i32* 指针，WASM 在此写入响应数据长度
//! // 返回：0 成功，< 0 错误码
//! actr_handle(req_ptr: i32, req_len: i32, resp_ptr_out: i32, resp_len_out: i32) -> i32
//! ```
//!
//! # 使用示例
//!
//! ```rust,ignore
//! use actr_runtime_wasm::entry;
//!
//! struct MyService;
//! // impl EchoServiceHandler for MyService { ... }
//!
//! // 使用 Default 初始化（要求 MyWorkload: Default）
//! entry!(EchoServiceWorkload<MyService>);
//!
//! // 或提供自定义初始化表达式
//! entry!(EchoServiceWorkload<MyService>, EchoServiceWorkload(MyService::new()));
//! ```

/// ABI 错误码（与 actr-runtime::wasm::abi::code 保持一致）
pub mod code {
    pub const SUCCESS: i32 = 0;
    pub const GENERIC_ERROR: i32 = -1;
    pub const INIT_FAILED: i32 = -2;
    pub const HANDLE_FAILED: i32 = -3;
    pub const ALLOC_FAILED: i32 = -4;
    pub const PROTOCOL_ERROR: i32 = -5;
}

/// 为 Workload 类型生成 WASM ABI 导出函数
///
/// # 参数
///
/// - `$workload_type`：实现了 `actr_framework::Workload + Send + Sync + 'static` 的类型
/// - `$init_expr`（可选）：构造 Workload 实例的表达式；省略时使用 `Default::default()`
///
/// # 生成的导出
///
/// - `actr_alloc(size: i32) -> i32`
/// - `actr_free(ptr: i32, size: i32)`
/// - `actr_init(config_ptr: i32, config_len: i32) -> i32`
/// - `actr_handle(req_ptr: i32, req_len: i32, resp_ptr_out: i32, resp_len_out: i32) -> i32`
#[macro_export]
macro_rules! entry {
    // 单参数形式：使用 Default::default() 初始化
    ($workload_type:ty) => {
        $crate::entry!($workload_type, <$workload_type as Default>::default());
    };

    // 双参数形式：使用自定义初始化表达式
    ($workload_type:ty, $init_expr:expr) => {
        // WASM 是单线程，static mut 访问安全
        static mut __ACTR_WORKLOAD: Option<$workload_type> = None;

        /// 分配 WASM 线性内存（宿主在写入数据前调用）
        #[unsafe(no_mangle)]
        pub extern "C" fn actr_alloc(size: i32) -> i32 {
            let layout = std::alloc::Layout::from_size_align(size as usize, 1)
                .expect("invalid layout");
            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                $crate::abi::code::ALLOC_FAILED
            } else {
                ptr as i32
            }
        }

        /// 释放 WASM 线性内存（宿主读写完毕后调用）
        #[unsafe(no_mangle)]
        pub extern "C" fn actr_free(ptr: i32, size: i32) {
            if ptr == 0 || size <= 0 {
                return;
            }
            let layout = std::alloc::Layout::from_size_align(size as usize, 1)
                .expect("invalid layout");
            unsafe { std::alloc::dealloc(ptr as *mut u8, layout) };
        }

        /// 初始化 actor（宿主在首次 actr_handle 调用前调用）
        ///
        /// `config_ptr/len`：JSON 编码的 WasmActorConfig（保留供未来扩展）
        #[unsafe(no_mangle)]
        pub extern "C" fn actr_init(_config_ptr: i32, _config_len: i32) -> i32 {
            let workload: $workload_type = $init_expr;
            unsafe {
                __ACTR_WORKLOAD = Some(workload);
            }
            $crate::abi::code::SUCCESS
        }

        /// 处理一条 RPC 请求
        ///
        /// - `req_ptr/len`：protobuf 编码的 `RpcEnvelope`
        /// - `resp_ptr_out`：宿主提供的 `i32*`，WASM 在此写入响应缓冲区地址
        /// - `resp_len_out`：宿主提供的 `i32*`，WASM 在此写入响应数据长度
        #[unsafe(no_mangle)]
        pub extern "C" fn actr_handle(
            req_ptr: i32,
            req_len: i32,
            resp_ptr_out: i32,
            resp_len_out: i32,
        ) -> i32 {
            use actr_protocol::prost::Message as ProstMessage;
            use actr_framework::{MessageDispatcher, Workload};

            // 读取请求 envelope
            let req_bytes: &[u8] =
                unsafe { std::slice::from_raw_parts(req_ptr as *const u8, req_len as usize) };

            let envelope = match actr_protocol::RpcEnvelope::decode(req_bytes) {
                Ok(e) => e,
                Err(_) => return $crate::abi::code::PROTOCOL_ERROR,
            };

            // 构建 WasmContext（从宿主获取当前调用的上下文数据）
            let ctx = match $crate::context::WasmContext::from_host() {
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
            let resp_result =
                $crate::executor::block_on(Dispatcher::dispatch(workload, envelope, &ctx));

            let resp_bytes = match resp_result {
                Ok(b) => b,
                Err(_) => return $crate::abi::code::HANDLE_FAILED,
            };

            // 在 WASM 线性内存中分配响应缓冲区，返回给宿主
            let resp_len = resp_bytes.len();
            let layout = std::alloc::Layout::from_size_align(resp_len.max(1), 1)
                .expect("invalid layout");
            let resp_ptr = unsafe { std::alloc::alloc(layout) };
            if resp_ptr.is_null() {
                return $crate::abi::code::ALLOC_FAILED;
            }

            // 将响应数据写入 WASM 线性内存
            unsafe {
                std::ptr::copy_nonoverlapping(resp_bytes.as_ptr(), resp_ptr, resp_len);
                // 将响应缓冲区地址和长度写入宿主提供的输出指针
                *(resp_ptr_out as *mut i32) = resp_ptr as i32;
                *(resp_len_out as *mut i32) = resp_len as i32;
            }

            $crate::abi::code::SUCCESS
        }
    };
}
