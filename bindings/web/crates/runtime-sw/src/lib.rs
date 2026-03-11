//! Actor-RTC Service Worker Runtime
//!
//! Service Worker 端的 Actor 运行时，负责：
//! - State Path：Mailbox + Scheduler + Actor 执行
//! - WebSocket Lane：与服务器的 WebSocket 连接
//! - PostMessage Lane：与 DOM 的通信通道
//! - 流控制：OpenStream/CloseStream 控制平面

pub mod actr_ref;
pub mod context;
pub mod error_handler;
pub mod inbound;
pub mod lifecycle;
pub mod outbound;
pub mod runtime;
pub mod system;
pub mod trace;
pub mod transport;
pub mod web_context; // Web 专用 Context trait
pub mod webrtc_recovery;
pub use actr_framework::Workload;

pub use actr_mailbox_web::{
    IndexedDbMailbox, Mailbox, MailboxStats, MessagePriority, MessageRecord,
};
pub use actr_ref::ActrRef;
pub use actr_web_common::{
    ConnType, ConnectionState, ConnectionStrategy, Dest, ErrorCategory, ErrorContext, ErrorReport,
    ErrorSeverity, MessageFormat, PayloadType, TransportStats, WebError, WebResult,
};
pub use context::RuntimeContext;
pub use error_handler::{
    ErrorCallback, ErrorStats, SwErrorHandler, get_global_error_handler, init_global_error_handler,
};
pub use inbound::{InboundPacketDispatcher, MailboxMessageHandler, MailboxProcessor};
pub use lifecycle::SwLifecycleManager;
pub use outbound::{Gate, HostGate, PeerGate};
pub use runtime::{
    ServiceHandlerFn, handle_dom_control, handle_dom_fast_path, handle_dom_webrtc_event,
    init_global, register_client, register_datachannel_port, register_service_handler,
    unregister_client,
};
pub use system::System;
pub use transport::{
    DataLane, DestTransport, PeerTransport, PostMessageLaneBuilder, SwTransport,
    WebSocketLaneBuilder, WebWireBuilder, WireBuilder, WireHandle, WirePool,
};
pub use web_context::RuntimeBridge; // 导出 RuntimeBridge trait
pub use web_context::WebContext; // 导出 Web Context trait

// Re-export actr_protocol so downstream crates don't need a direct dependency
pub use actr_protocol;
pub use webrtc_recovery::{RecoveryStatus, WebRtcRecoveryManager};

/// 初始化 Service Worker Runtime
///
/// 应该在 Service Worker 激活时调用一次
///
/// 注意：错误处理器需要 WirePool 实例，应该在创建 WirePool 后调用
/// `init_global_error_handler(wire_pool)` 单独初始化
// #[wasm_bindgen]
// pub fn init_sw_runtime() {
//     // 设置 panic hook
//     console_error_panic_hook::set_once();

//     // 初始化日志
//     wasm_logger::init(wasm_logger::Config::default());

//     // 初始化生命周期管理
//     let lifecycle = SwLifecycleManager::new();
//     if let Err(e) = lifecycle.init() {
//         log::error!("Failed to initialize lifecycle manager: {:?}", e);
//     }

//     console::log_1(&"Actor-RTC Service Worker Runtime initialized".into());
// }

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // #[wasm_bindgen_test]
    // fn test_init() {
    //     init_sw_runtime();
    // }
}
