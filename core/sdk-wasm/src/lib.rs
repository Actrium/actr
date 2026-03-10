//! Actor WASM guest SDK
//!
//! 供编译到 `wasm32-unknown-unknown` 的 actor 使用。屏蔽线性内存 ABI 细节，
//! 开发者只需实现 [`WasmActor`] trait 并调用 [`entry!`] 宏注册即可。
//!
//! # 使用示例
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

/// WASM actor 开发者需实现的 trait
///
/// 对应 Host 侧调用的两个生命周期：
/// - `init`：actor 首次启动，接收凭证配置
/// - `handle`：每条消息到来时被调用，同步处理后返回响应
///
/// 两个方法均为**同步**——异步 IO 由 Host（Hyper 层）全权管理，
/// guest 内部只做纯计算。
pub trait WasmActor: Sized + 'static {
    /// 初始化 actor，返回 actor 实例
    ///
    /// Host 保证在任何 `handle` 调用之前调用且仅调用一次。
    fn init(config: ActorConfig) -> Self;

    /// 处理一条请求，返回响应字节
    ///
    /// - `request`：Host 写入的原始请求字节（通常为 protobuf 编码）
    /// - 返回值：响应字节，空 `Vec` 表示无响应（fire-and-forget）
    fn handle(&mut self, request: &[u8]) -> Vec<u8>;
}

/// 注册 actor 类型，生成所有必要的 ABI 导出函数
///
/// 展开后自动生成：
/// - `actr_init(config_ptr, config_len) -> i32`
/// - `actr_handle(req_ptr, req_len, resp_ptr_out, resp_len_out) -> i32`
/// - `actr_alloc(size) -> i32`
/// - `actr_free(ptr, size)`
///
/// # 示例
///
/// ```rust,ignore
/// entry!(MyActor);
/// ```
#[macro_export]
macro_rules! entry {
    ($actor:ty) => {
        // ABI 错误码，与 Host 侧 abi::code 保持一致
        const __SUCCESS: i32 = 0;
        const __GENERIC_ERROR: i32 = -1;
        const __INIT_FAILED: i32 = -2;
        const __HANDLE_FAILED: i32 = -3;
        const __PROTOCOL_ERROR: i32 = -5;

        static mut __ACTR_INSTANCE: Option<$actor> = None;

        /// actr_init：Host 调用一次，传入 JSON 编码的 ActorConfig
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

        /// actr_handle：Host 每次分发消息时调用
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
                // 将响应复制到独立分配的缓冲区，由 Host 读取后调用 actr_free 释放
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

        /// actr_alloc：Host 调用以在 WASM 线性内存中分配缓冲区
        #[no_mangle]
        pub unsafe extern "C" fn actr_alloc(size: i32) -> i32 {
            if size <= 0 {
                return 0;
            }
            let layout =
                core::alloc::Layout::from_size_align(size as usize, 1).unwrap();
            std::alloc::alloc(layout) as i32
        }

        /// actr_free：Host 调用以释放之前分配的缓冲区
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
