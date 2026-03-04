//! Actor-RTC DOM Runtime
//!
//! DOM 端的 Actor 运行时，负责：
//! - Fast Path：Stream Handler Registry + MediaFrame Handler Registry
//! - WebRTC Lanes：DataChannel + MediaTrack
//! - PostMessage Lane：与 Service Worker 的通信通道
//! - Keepalive：防止 Service Worker 被浏览器回收

use wasm_bindgen::prelude::*;
use web_sys::console;

pub mod error_reporter;
pub mod fastpath;
pub mod inbound;
pub mod keepalive;
pub mod lifecycle;
pub mod system;
pub mod transport;
pub mod webrtc;

pub use actr_web_common::{
    ConnectionState, ConnectionStrategy, Dest, ErrorCategory, ErrorContext, ErrorSeverity,
    MessageFormat, PayloadType, TransportStats, WebError, WebResult,
};
pub use error_reporter::{DomErrorReporter, get_global_error_reporter, init_global_error_reporter};
pub use fastpath::{
    MediaFrameCallback, MediaFrameHandlerRegistry, StreamCallback, StreamHandlerRegistry,
};
pub use inbound::{DomInboundDispatcher, WebRtcDataChannelReceiver};
pub use keepalive::ServiceWorkerKeepalive;
pub use lifecycle::DomLifecycleManager;
pub use system::DomSystem;
pub use transport::{DataLane, DomTransport, PostMessageLaneBuilder};
pub use webrtc::WebRtcCoordinator;

/// 初始化 DOM Runtime
///
/// 应该在页面加载时调用一次
#[wasm_bindgen]
pub fn init_dom_runtime() {
    // 设置 panic hook
    console_error_panic_hook::set_once();

    // 初始化日志
    wasm_logger::init(wasm_logger::Config::default());

    // 初始化错误报告器
    let _error_reporter = init_global_error_reporter();
    log::info!("Error reporter initialized");

    // 初始化生命周期管理
    let lifecycle = DomLifecycleManager::new();
    if let Err(e) = lifecycle.init() {
        log::error!("Failed to initialize lifecycle manager: {:?}", e);
    }

    console::log_1(&"Actor-RTC DOM Runtime initialized".into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_init() {
        init_dom_runtime();
    }
}
