//! WASM Host-Guest ABI 定义
//!
//! 本 ABI 设计为**与运行时无关**，兼容 Wasmtime（桌面/服务端）和 WAMR（移动/嵌入式），
//! 仅使用 WASM 核心规范（线性内存 + 函数调用），不依赖 Component Model。
//!
//! # Guest 必须实现的导出函数
//!
//! ```text
//! /// 初始化 actor（接收 JSON 编码的配置，见 WasmActorConfig）
//! actr_init(config_ptr: i32, config_len: i32) -> i32
//!
//! /// 处理一条请求消息（protobuf 编码的 RpcEnvelope）
//! /// - req_ptr / req_len：请求数据在线性内存中的起始地址和长度
//! /// - resp_ptr_out：Host 分配的 i32 指针，WASM 在此写入响应数据的地址
//! /// - resp_len_out：Host 分配的 i32 指针，WASM 在此写入响应数据的长度
//! /// Host 读完响应后调用 actr_free(resp_ptr, resp_len) 释放 WASM 分配的内存
//! actr_handle(req_ptr: i32, req_len: i32,
//!             resp_ptr_out: i32, resp_len_out: i32) -> i32
//!
//! /// 在 WASM 线性内存中分配内存（供 Host 写入数据前调用）
//! /// 返回分配的指针，0 表示分配失败
//! actr_alloc(size: i32) -> i32
//!
//! /// 释放 WASM 线性内存（Host 完成读写后调用）
//! actr_free(ptr: i32, size: i32)
//! ```
//!
//! # 数据写入协议（Host → WASM）
//!
//! ```text
//! 1. host 调用 actr_alloc(size) → ptr
//! 2. host 将数据写入 wasm_memory[ptr..ptr+size]
//! 3. host 调用目标函数并传入 ptr, size
//! 4. host 调用 actr_free(ptr, size) 释放内存
//! ```
//!
//! # 数据读取协议（WASM → Host）
//!
//! ```text
//! 1. host 在自身栈上分配两个 i32 变量：resp_ptr_out, resp_len_out
//! 2. host 将它们的地址通过 actr_alloc 分配的临时区写入 WASM 内存，
//!    并把地址传给 actr_handle
//! 3. WASM 在 actr_handle 内部分配响应内存，将 ptr/len 写入 resp_ptr_out/resp_len_out
//! 4. host 读取 resp_ptr_out/resp_len_out，从 WASM 内存取出响应数据
//! 5. host 调用 actr_free(resp_ptr, resp_len) 释放 WASM 响应内存
//! ```
//!
//! # 错误码（所有返回 i32 的函数）
//!
//! | 值   | 含义                     |
//! |-----|--------------------------|
//! | 0   | 成功                     |
//! | -1  | 通用错误                 |
//! | -2  | 初始化失败               |
//! | -3  | 消息处理失败             |
//! | -4  | 内存分配失败             |
//! | -5  | 协议错误（消息格式非法） |

/// WASM 导出函数名
pub const EXPORT_INIT: &str = "actr_init";
pub const EXPORT_HANDLE: &str = "actr_handle";
pub const EXPORT_ALLOC: &str = "actr_alloc";
pub const EXPORT_FREE: &str = "actr_free";
pub const EXPORT_MEMORY: &str = "memory";

/// ABI 错误码
pub mod code {
    pub const SUCCESS: i32 = 0;
    pub const GENERIC_ERROR: i32 = -1;
    pub const INIT_FAILED: i32 = -2;
    pub const HANDLE_FAILED: i32 = -3;
    pub const ALLOC_FAILED: i32 = -4;
    pub const PROTOCOL_ERROR: i32 = -5;
}

/// 将 ABI 错误码转换为可读描述
pub fn describe_error_code(code: i32) -> &'static str {
    match code {
        self::code::SUCCESS => "success",
        self::code::GENERIC_ERROR => "generic error",
        self::code::INIT_FAILED => "initialization failed",
        self::code::HANDLE_FAILED => "message handling failed",
        self::code::ALLOC_FAILED => "memory allocation failed",
        self::code::PROTOCOL_ERROR => "protocol error (malformed message)",
        _ => "unknown error",
    }
}

/// WasmActorConfig - 初始化时传给 actr_init 的 JSON 结构
///
/// Guest 端收到后解析并初始化内部状态。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WasmActorConfig {
    /// Actor 类型（manufacturer:name:version）
    pub actr_type: String,

    /// AID 凭证（protobuf bytes，base64 编码）
    pub credential_b64: String,

    /// Actor ID（protobuf bytes，base64 编码）
    pub actor_id_b64: String,

    /// Realm ID
    pub realm_id: u32,
}
