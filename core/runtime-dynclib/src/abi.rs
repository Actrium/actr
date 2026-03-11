//! ABI 错误码定义
//!
//! 与 `actr-hyper::executor::error_code` 保持一致，供 `entry_dynclib!` 宏
//! 及 `DynclibContext` 内部使用。

/// ABI 错误码
pub mod code {
    /// 操作成功
    pub const SUCCESS: i32 = 0;
    /// 通用不可恢复错误
    pub const GENERIC_ERROR: i32 = -1;
    /// 初始化失败
    pub const INIT_FAILED: i32 = -2;
    /// 消息处理失败
    pub const HANDLE_FAILED: i32 = -3;
    /// 协议 / 编解码错误
    pub const PROTOCOL_ERROR: i32 = -5;
}
