//! HostVTable — 宿主回调函数指针表
//!
//! 宿主 (Hyper) 在 `actr_init` 时传入此表，guest 将其缓存到 thread-local
//! 中，后续 `DynclibContext` 通过函数指针完成 RPC、发现等操作。
//!
//! 与 WASM 的 host imports 不同，dynclib 与宿主共享地址空间，
//! 因此不需要 alloc/free 中转——直接传递指针即可。

/// 宿主回调函数指针表
///
/// # Safety
///
/// - 所有函数指针在 `actr_init` 到进程退出期间保持有效
/// - 宿主保证线程安全性：同一 actor 实例不会被并发调用
/// - `*mut *mut u8` 输出指针指向宿主分配的内存，guest 必须通过 `free_host_buf` 释放
#[repr(C)]
pub struct HostVTable {
    /// RPC 调用并等待响应
    ///
    /// `call(route_key_ptr, route_key_len, dest_ptr, dest_len,
    ///       payload_ptr, payload_len, resp_ptr_out, resp_len_out) -> error_code`
    ///
    /// 宿主在 `resp_ptr_out` / `resp_len_out` 写入响应缓冲区地址和长度。
    /// guest 使用完毕后必须调用 `free_host_buf` 释放。
    pub call: unsafe extern "C" fn(
        *const u8,
        usize,
        *const u8,
        usize,
        *const u8,
        usize,
        *mut *mut u8,
        *mut usize,
    ) -> i32,

    /// 单向消息 (fire-and-forget)
    ///
    /// `tell(route_key_ptr, route_key_len, dest_ptr, dest_len,
    ///       payload_ptr, payload_len) -> error_code`
    pub tell:
        unsafe extern "C" fn(*const u8, usize, *const u8, usize, *const u8, usize) -> i32,

    /// 服务发现
    ///
    /// `discover(type_ptr, type_len, resp_ptr_out, resp_len_out) -> error_code`
    ///
    /// 宿主在 `resp_ptr_out` / `resp_len_out` 写入 ActrId protobuf 编码。
    /// guest 使用完毕后必须调用 `free_host_buf` 释放。
    pub discover: unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32,

    /// 获取当前 actor 的 ActrId
    ///
    /// `self_id(buf_ptr_out, buf_len_out) -> error_code`
    ///
    /// 宿主写入 protobuf 编码的 ActrId，guest 通过 `free_host_buf` 释放。
    pub self_id: unsafe extern "C" fn(*mut *mut u8, *mut usize) -> i32,

    /// 获取调用方 ActrId
    ///
    /// `caller_id(buf_ptr_out, buf_len_out) -> i32`
    ///
    /// 返回 0 表示有调用方，1 表示无调用方（系统内部调用）。
    /// 有调用方时宿主写入 protobuf 编码的 ActrId，guest 通过 `free_host_buf` 释放。
    pub caller_id: unsafe extern "C" fn(*mut *mut u8, *mut usize) -> i32,

    /// 获取当前请求 ID
    ///
    /// `request_id(buf_ptr_out, buf_len_out) -> error_code`
    ///
    /// 宿主写入 UTF-8 编码的请求 ID，guest 通过 `free_host_buf` 释放。
    pub request_id: unsafe extern "C" fn(*mut *mut u8, *mut usize) -> i32,

    /// 释放宿主分配的缓冲区
    ///
    /// `free_host_buf(ptr, len)`
    ///
    /// 所有由宿主通过 `*_out` 指针返回的缓冲区，guest 使用完毕后必须调用此函数释放。
    pub free_host_buf: unsafe extern "C" fn(*mut u8, usize),
}
