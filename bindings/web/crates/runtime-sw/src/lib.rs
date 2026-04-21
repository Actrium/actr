//! Actor-RTC Service Worker Runtime
//!
//! Actor runtime for the Service Worker side. It is responsible for:
//! - The State Path: mailbox, scheduler, and actor execution
//! - The WebSocket lane for server communication
//! - The PostMessage lane for DOM communication
//! - Stream control through the OpenStream/CloseStream control plane

pub mod actr_ref;
pub mod context;
pub mod error_handler;
pub mod guest_bridge;
pub mod guest_bridge_wit;
pub mod inbound;
pub mod lifecycle;
pub mod outbound;
pub mod package;
pub mod runtime;
pub mod system;
pub mod trace;
pub mod transport;
pub mod web_context; // Web-specific Context trait
pub mod webrtc_recovery;
pub mod workload;
pub use actr_framework::Workload;
pub use actr_platform_web::WebPlatformProvider;

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
pub use guest_bridge::{
    encode_guest_init_payload, guest_host_invoke_async, register_guest_workload,
};
pub use inbound::{InboundPacketDispatcher, MailboxMessageHandler, MailboxProcessor};
pub use lifecycle::SwLifecycleManager;
pub use outbound::{Gate, HostGate, PeerGate};
pub use runtime::{
    handle_dom_control, handle_dom_fast_path, handle_dom_webrtc_event, init_global,
    register_client, register_datachannel_port, register_workload, unregister_client,
};
pub use system::System;
pub use transport::{
    DataLane, DestTransport, PeerTransport, PostMessageLaneBuilder, SwTransport,
    WebSocketLaneBuilder, WebWireBuilder, WireBuilder, WireHandle, WirePool,
};
pub use web_context::RuntimeBridge; // Re-export RuntimeBridge trait
pub use web_context::WebContext;
pub use workload::{ServiceHandlerFn, WasmWorkload}; // Re-export WebContext trait

// Re-export actr_protocol so downstream crates don't need a direct dependency
pub use actr_protocol;
pub use webrtc_recovery::{RecoveryStatus, WebRtcRecoveryManager};

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);
}
