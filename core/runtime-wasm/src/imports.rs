//! Host import 声明
//!
//! 在 WASM 模块中通过 `extern "C"` 声明的宿主函数（由 `actr-runtime` 提供实现）。
//!
//! # 内存约定
//!
//! - 所有指针指向 WASM 线性内存中的地址（i32 偏移量）
//! - 宿主通过 `actr_alloc` 分配输入缓冲区，客体写入后通过 `actr_free` 释放
//! - 客体通过参数传入输出缓冲区的指针和最大长度，宿主直接写入
//!
//! # Asyncify 协议
//!
//! `actr_host_call` / `actr_host_call_raw` 支持 asyncify 透明挂起/恢复：
//! - **Normal 模式**：宿主在 import 回调内调用 `asyncify_start_unwind`，WASM 保存调用栈后返回
//! - **Rewinding 模式**：宿主重新进入 WASM，调用 `asyncify_stop_rewind`，import 直接返回实际结果
//! - 从客体侧看，import 调用与普通同步函数调用无异

unsafe extern "C" {
    /// 获取当前 Actor 的 ActrId（protobuf 编码）
    ///
    /// 宿主将 ActrId bytes 写入 `[out_ptr, out_ptr + out_max)`
    /// 返回实际写入字节数，0 表示错误
    pub fn actr_host_self_id(out_ptr: i32, out_max: i32) -> i32;

    /// 获取调用方 Actor 的 ActrId（protobuf 编码）
    ///
    /// 宿主将调用方 ActrId bytes 写入输出缓冲区
    /// 返回实际写入字节数，-1 表示无调用方（系统调用，如生命周期钩子）
    pub fn actr_host_caller_id(out_ptr: i32, out_max: i32) -> i32;

    /// 获取当前请求 ID（UTF-8 字符串）
    ///
    /// 宿主将请求 ID 写入输出缓冲区
    /// 返回实际写入字节数
    pub fn actr_host_request_id(out_ptr: i32, out_max: i32) -> i32;

    /// 发送 RPC 请求并等待响应（asyncify 透明挂起/恢复）
    ///
    /// 参数：
    /// - `route_key_ptr/len`：路由键（UTF-8）
    /// - `dest_ptr/len`：目标 Dest 编码（见 [`crate::context::encode_dest`]）
    /// - `payload_ptr/len`：protobuf 编码的请求 payload
    /// - `out_ptr/out_max`：响应输出缓冲区（客体预分配）
    /// - `out_len_ptr`：宿主写入实际响应长度的地址（i32*）
    ///
    /// 返回：0 成功，< 0 见 [`crate::abi::code`]
    pub fn actr_host_call(
        route_key_ptr: i32,
        route_key_len: i32,
        dest_ptr: i32,
        dest_len: i32,
        payload_ptr: i32,
        payload_len: i32,
        out_ptr: i32,
        out_max: i32,
        out_len_ptr: i32,
    ) -> i32;

    /// 发送单向消息（fire-and-forget，不等待响应）
    ///
    /// 返回：0 成功，< 0 见 [`crate::abi::code`]
    pub fn actr_host_tell(
        route_key_ptr: i32,
        route_key_len: i32,
        dest_ptr: i32,
        dest_len: i32,
        payload_ptr: i32,
        payload_len: i32,
    ) -> i32;

    /// 原始 RPC 调用（按 ActrId 路由，不经 Dest 解析）
    ///
    /// 返回：0 成功，< 0 见 [`crate::abi::code`]
    pub fn actr_host_call_raw(
        route_key_ptr: i32,
        route_key_len: i32,
        target_ptr: i32,
        target_len: i32,
        payload_ptr: i32,
        payload_len: i32,
        out_ptr: i32,
        out_max: i32,
        out_len_ptr: i32,
    ) -> i32;

    /// 通过 Actor 类型发现路由候选（Signaling 服务发现）
    ///
    /// - `type_ptr/len`：ActrType protobuf 编码
    /// - `out_ptr/out_max`：输出 ActrId protobuf 编码的缓冲区
    ///
    /// 返回实际写入字节数，< 0 错误码
    pub fn actr_host_discover(type_ptr: i32, type_len: i32, out_ptr: i32, out_max: i32) -> i32;
}
