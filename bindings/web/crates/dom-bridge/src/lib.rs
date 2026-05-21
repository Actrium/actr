//! Actor-RTC DOM Runtime
//!
//! Actor runtime for the DOM side. It is responsible for:
//! - Fast path support through the stream handler and media frame registries
//! - WebRTC lanes for DataChannel and MediaTrack
//! - The PostMessage lane used to communicate with the Service Worker
//! - Keepalive logic so the browser does not reclaim the Service Worker

use wasm_bindgen::prelude::*;
use web_sys::console;

pub mod error_reporter;
pub mod fastpath;
pub mod inbound;
pub mod keepalive;
pub mod lifecycle;
pub mod system;
pub mod transport;

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

/// Initialize the DOM runtime.
///
/// Call this once when the page loads.
#[wasm_bindgen]
pub fn init_dom_runtime() {
    // Install the panic hook.
    console_error_panic_hook::set_once();

    // Initialize logging.
    wasm_logger::init(wasm_logger::Config::default());

    // Initialize the error reporter.
    let _error_reporter = init_global_error_reporter();
    log::info!("Error reporter initialized");

    // Initialize lifecycle management.
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
