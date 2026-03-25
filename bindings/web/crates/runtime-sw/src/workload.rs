//! Runtime workload abstraction for the Web platform.
//!
//! Analogous to `core/hyper::workload::Workload`, but only supports the
//! WASM backend since the Web runtime runs entirely in a browser Service Worker.
//!
//! # Design
//!
//! In the native runtime (`core/hyper`), `Workload` is an enum with `None`,
//! `Native`, `Wasm`, and `DynClib` variants. The Web runtime only ever runs
//! inside WASM, so there is a single `WasmWorkload` struct. The user
//! constructs it from a `ServiceHandlerFn` and registers it with the SW
//! runtime via [`register_workload`](crate::register_workload).

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use crate::context::RuntimeContext;

/// Handler function signature for dispatching RPC requests.
///
/// Given `(route_key, request_bytes, context)`, returns a future that resolves
/// to the response bytes or an error string.
///
/// The handler dispatches based on `route_key` prefix to local or remote handlers.
/// For remote calls, the handler uses `ctx.call_raw()` / `ctx.discover()`.
///
/// # Parameters
/// - `route_key`: Full route key (e.g., `"echo.EchoService.Echo"`)
/// - `request_bytes`: Serialized protobuf request payload
/// - `ctx`: WebContext providing communication capabilities (call_raw, discover, etc.)
pub type ServiceHandlerFn = Rc<
    dyn Fn(
        &str,
        &[u8],
        Rc<RuntimeContext>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>>>>,
>;

/// WASM workload for the Service Worker runtime.
///
/// Wraps a [`ServiceHandlerFn`] that dispatches RPC requests to business logic.
/// This is the Web equivalent of `core/hyper`'s `Workload::Wasm` variant.
///
/// # Example
///
/// ```ignore
/// use std::rc::Rc;
/// use actr_runtime_sw::{WasmWorkload, register_workload};
///
/// let workload = WasmWorkload::new(Rc::new(|route_key, bytes, ctx| {
///     Box::pin(async move {
///         match route_key {
///             "echo.EchoService.Echo" => handle_echo(bytes, ctx).await,
///             _ => Err(format!("Unknown route: {}", route_key)),
///         }
///     })
/// }));
/// register_workload(workload);
/// ```
#[derive(Clone)]
pub struct WasmWorkload {
    handler: ServiceHandlerFn,
}

impl WasmWorkload {
    /// Create a new WASM workload from a handler function.
    pub fn new(handler: ServiceHandlerFn) -> Self {
        Self { handler }
    }

    /// Dispatch an RPC request through the workload handler.
    pub async fn dispatch(
        &self,
        route_key: &str,
        request_bytes: &[u8],
        ctx: Rc<RuntimeContext>,
    ) -> Result<Vec<u8>, String> {
        (self.handler)(route_key, request_bytes, ctx).await
    }

    /// Get a reference to the underlying handler function.
    pub fn handler(&self) -> &ServiceHandlerFn {
        &self.handler
    }
}

impl std::fmt::Debug for WasmWorkload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmWorkload").finish_non_exhaustive()
    }
}
