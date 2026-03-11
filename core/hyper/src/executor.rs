//! ExecutorAdapter - unified dispatch interface for dynamic actor runtimes
//!
//! Shared abstraction for WASM and dynclib actor execution.
//! Types defined here (`DispatchContext`, `PendingCall`, `IoResult`) are used by
//! all executor backends and the drive loop in `ActrNode::handle_incoming`.

use actr_protocol::ActrId;
use std::future::Future;
use std::pin::Pin;

/// Dispatch context passed to executor for each request
#[derive(Debug, Default)]
pub struct DispatchContext {
    pub self_id: ActrId,
    pub caller_id: Option<ActrId>,
    pub request_id: String,
}

/// Pending outbound call from guest actor
#[derive(Debug)]
pub enum PendingCall {
    /// RPC call with response (routed via Dest)
    Call {
        route_key: String,
        dest_bytes: Vec<u8>,
        payload: Vec<u8>,
    },
    /// Fire-and-forget message
    Tell {
        route_key: String,
        dest_bytes: Vec<u8>,
        payload: Vec<u8>,
    },
    /// Service discovery (by ActrType)
    Discover { type_bytes: Vec<u8> },
    /// Raw RPC call (direct ActrId routing)
    CallRaw {
        route_key: String,
        target_bytes: Vec<u8>,
        payload: Vec<u8>,
    },
}

/// Result of an outbound IO operation
#[derive(Debug)]
pub enum IoResult {
    /// Response bytes from Call / CallRaw / Discover
    Bytes(Vec<u8>),
    /// Tell completed (no response data)
    Done,
    /// Error code
    Error(i32),
}

/// Standard error codes shared across executor backends
pub mod error_code {
    /// Generic unrecoverable error
    pub const GENERIC_ERROR: i32 = -1;
    /// Protocol / encoding error (malformed message)
    pub const PROTOCOL_ERROR: i32 = -5;
}

/// Type alias for the call executor closure
pub type CallExecutorFn =
    Box<dyn Fn(PendingCall) -> Pin<Box<dyn Future<Output = IoResult> + Send>> + Send + Sync>;

/// Result type for ExecutorAdapter dispatch
pub type DispatchResult = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;

/// Unified dispatch interface for dynamic actor runtimes (WASM, dynclib, etc.)
///
/// Each executor backend (e.g. `WasmInstance`) implements this trait so that
/// `ActrNode` can dispatch requests through a type-erased `Box<dyn ExecutorAdapter>`.
///
/// # Concurrency contract
///
/// A single executor instance represents one logical guest actor instance.
/// `ActrNode` serializes calls to that executor with a `Mutex`, so implementors
/// may rely on `dispatch()` never being entered concurrently for the same
/// instance. Hosts that want multiple actors of the same type must create
/// multiple executor instances.
pub trait ExecutorAdapter: Send {
    /// Dispatch a request through the guest actor
    fn dispatch<'a>(
        &'a mut self,
        request_bytes: &[u8],
        ctx: DispatchContext,
        call_executor: &'a CallExecutorFn,
    ) -> Pin<Box<dyn Future<Output = DispatchResult> + Send + 'a>>;
}
