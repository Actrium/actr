use std::any::Any;

use actr_framework::{Bytes, Context as FrameworkContext, Workload};
use actr_protocol::{ActorResult, ActrId, ProtocolError, RpcEnvelope};
use actr_runtime::context::RuntimeContext;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyString};

use crate::runtime::ContextPy;
use crate::types::ActrIdPy;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Shared helper
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Downcast generic Context to RuntimeContext.
fn downcast_ctx<C: FrameworkContext>(ctx: &C) -> Result<&RuntimeContext, ProtocolError> {
    (ctx as &dyn Any)
        .downcast_ref::<RuntimeContext>()
        .ok_or_else(|| ProtocolError::TransportError("Context is not RuntimeContext".into()))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Macros — reduce per-hook boilerplate
//
// These macros generate the *body* of each hook, called
// from within the `#[async_trait] impl Workload` block.
//
// All hooks use `asyncio.run_coroutine_threadsafe(coro, event_loop)`
// because hooks may be invoked from Rust background threads (e.g.
// WebRTC coordinator) where no Python event loop is running.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Invoke a Python hook with `(ctx,)` args. Returns `Ok(())` if method absent.
macro_rules! py_hook_ctx {
    ($self:ident, $method:literal, $ctx:expr) => {{
        let runtime_ctx = downcast_ctx($ctx)?;
        if !$self.has_method($method)? {
            return Ok(());
        }
        let ctx_obj = $self.wrap_context(runtime_ctx)?;
        let event_loop = $self.require_event_loop()?;

        Python::attach(|py| -> PyResult<()> {
            let obj = $self.py_obj.bind(py);
            let coro = obj.call_method1($method, (ctx_obj.clone_ref(py),))?;
            let asyncio = py.import("asyncio")?;
            let future = asyncio
                .getattr("run_coroutine_threadsafe")?
                .call1((coro, event_loop.bind(py)))?;
            future.getattr("result")?.call0()?;
            Ok(())
        })
        .map_err(|e| {
            ProtocolError::TransportError(format!(concat!("Python ", $method, " failed: {}"), e))
        })?;
        Ok(())
    }};
}

/// Invoke a Python hook with `(ctx_or_none,)` args.
macro_rules! py_hook_optional_ctx {
    ($self:ident, $method:literal, $ctx:expr) => {{
        if !$self.has_method($method)? {
            return Ok(());
        }
        let ctx_obj: Py<PyAny> = match $ctx {
            Some(c) => $self.wrap_context(downcast_ctx(c)?)?,
            None => Python::attach(|py| py.None().into()),
        };
        let event_loop = $self.require_event_loop()?;

        Python::attach(|py| -> PyResult<()> {
            let obj = $self.py_obj.bind(py);
            let coro = obj.call_method1($method, (ctx_obj.clone_ref(py),))?;
            let asyncio = py.import("asyncio")?;
            let future = asyncio
                .getattr("run_coroutine_threadsafe")?
                .call1((coro, event_loop.bind(py)))?;
            future.getattr("result")?.call0()?;
            Ok(())
        })
        .map_err(|e| {
            ProtocolError::TransportError(format!(concat!("Python ", $method, " failed: {}"), e))
        })?;
        Ok(())
    }};
}

/// Invoke a Python hook with `(ctx, peer_id)` args.
macro_rules! py_hook_ctx_dest {
    ($self:ident, $method:literal, $ctx:expr, $dest:expr) => {{
        let runtime_ctx = downcast_ctx($ctx)?;
        if !$self.has_method($method)? {
            return Ok(());
        }
        let ctx_obj = $self.wrap_context(runtime_ctx)?;
        let dest = $dest.clone();
        let event_loop = $self.require_event_loop()?;

        Python::attach(|py| -> PyResult<()> {
            let obj = $self.py_obj.bind(py);
            let peer_py = ActrIdPy::from_rust(dest.clone());
            let coro = obj.call_method1($method, (ctx_obj.clone_ref(py), peer_py))?;
            let asyncio = py.import("asyncio")?;
            let future = asyncio
                .getattr("run_coroutine_threadsafe")?
                .call1((coro, event_loop.bind(py)))?;
            future.getattr("result")?.call0()?;
            Ok(())
        })
        .map_err(|e| {
            ProtocolError::TransportError(format!(concat!("Python ", $method, " failed: {}"), e))
        })?;
        Ok(())
    }};
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PyWorkloadWrapper
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Workload wrapper that forwards lifecycle and dispatch to a Python object.
pub struct PyWorkloadWrapper {
    pub(crate) py_obj: Py<PyAny>,
    pub(crate) event_loop: Option<Py<PyAny>>,
}

impl PyWorkloadWrapper {
    pub fn new(obj: Py<PyAny>) -> PyResult<Self> {
        Ok(Self {
            py_obj: obj,
            event_loop: None,
        })
    }

    pub fn set_event_loop(&mut self, loop_obj: Py<PyAny>) {
        self.event_loop = Some(loop_obj);
    }

    fn make_context_py<'py>(
        &self,
        py: Python<'py>,
        ctx: &RuntimeContext,
    ) -> PyResult<Py<ContextPy>> {
        Py::new(py, ContextPy { inner: ctx.clone() })
    }

    /// Get Dispatcher object from Python Workload (if implemented)
    pub fn get_dispatcher(&self) -> Option<Py<PyAny>> {
        Python::attach(|py| -> PyResult<Option<Py<PyAny>>> {
            let obj = self.py_obj.bind(py);
            if obj.hasattr("get_dispatcher")? {
                if let Ok(dispatcher) = obj.call_method0("get_dispatcher") {
                    if let Ok(dispatcher_obj) = dispatcher.extract::<Py<PyAny>>() {
                        return Ok(Some(dispatcher_obj));
                    }
                }
            }
            Ok(None)
        })
        .ok()
        .flatten()
    }

    /// Check if the Python workload object has a given method.
    fn has_method(&self, name: &str) -> Result<bool, ProtocolError> {
        let name = name.to_string();
        Python::attach(|py| {
            let obj = self.py_obj.bind(py);
            Ok(obj.hasattr(&*name)?)
        })
        .map_err(|e: PyErr| ProtocolError::TransportError(format!("Python hasattr failed: {e}")))
    }

    /// Build the high-level `actr.Context` wrapper from a RuntimeContext.
    fn wrap_context(&self, runtime_ctx: &RuntimeContext) -> Result<Py<PyAny>, ProtocolError> {
        let ctx_py = Python::attach(|py| self.make_context_py(py, runtime_ctx)).map_err(|e| {
            ProtocolError::TransportError(format!("Failed to create ContextPy: {e}"))
        })?;

        Python::attach(|py| -> PyResult<Py<PyAny>> {
            let actr_module = py.import("actr")?;
            let ctx_class = actr_module.getattr("Context")?;
            let ctx_obj = ctx_class.call1((ctx_py.clone_ref(py),))?;
            Ok(ctx_obj.extract::<Py<PyAny>>()?)
        })
        .map_err(|e| ProtocolError::TransportError(format!("Failed to wrap Context: {e}")))
    }

    /// Get the stored event loop, or return an error.
    fn require_event_loop(&self) -> Result<&Py<PyAny>, ProtocolError> {
        self.event_loop.as_ref().ok_or_else(|| {
            ProtocolError::TransportError(
                "Event loop not set. Make sure to call attach() from within an async context."
                    .to_string(),
            )
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PyDispatcher
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct PyDispatcher;

impl PyDispatcher {
    async fn dispatch_with_dispatcher(
        workload: &PyWorkloadWrapper,
        dispatcher_obj: Py<PyAny>,
        runtime_ctx: &RuntimeContext,
        route_key: String,
        payload: Vec<u8>,
    ) -> ActorResult<Bytes> {
        let runtime_ctx_clone = runtime_ctx.clone();
        let route_key_clone = route_key.clone();
        let payload_clone = payload.clone();
        let workload_obj = Python::attach(|py| workload.py_obj.clone_ref(py));

        fn make_ctx_py(py: Python, ctx: &RuntimeContext) -> PyResult<Py<ContextPy>> {
            Py::new(py, ContextPy { inner: ctx.clone() })
        }

        let event_loop = workload.event_loop.as_ref().ok_or_else(|| {
            ProtocolError::TransportError(
                "Event loop not set. Make sure to call attach() from within an async context."
                    .to_string(),
            )
        })?;

        let result_obj = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let ctx_py = make_ctx_py(py, &runtime_ctx_clone).map_err(|e| {
                PyErr::from(PyValueError::new_err(format!(
                    "Failed to create ContextPy: {e}"
                )))
            })?;

            let dispatcher = dispatcher_obj.bind(py);
            let workload_py = workload_obj.bind(py);
            let ctx_obj = ctx_py.clone_ref(py);
            let route = PyString::new(py, &route_key_clone);
            let pay = PyBytes::new(py, &payload_clone);

            let coro = dispatcher.call_method1("dispatch", (workload_py, route, pay, ctx_obj))?;

            let asyncio = py.import("asyncio")?;
            let run_coroutine_threadsafe = asyncio.getattr("run_coroutine_threadsafe")?;
            let concurrent_future = run_coroutine_threadsafe.call1((coro, event_loop.bind(py)))?;

            let result_method = concurrent_future.getattr("result")?;
            let result = result_method.call0()?;
            Ok(result.into_any().into())
        })
        .map_err(|e| {
            ProtocolError::TransportError(format!("Python dispatcher.dispatch call failed: {e}"))
        })?;

        Python::attach(|py| {
            let bound = result_obj.bind(py);
            if let Ok(b) = bound.cast::<PyBytes>() {
                Ok(Bytes::from(b.as_bytes().to_vec()))
            } else if let Ok(v) = bound.extract::<Vec<u8>>() {
                Ok(Bytes::from(v))
            } else {
                Err(ProtocolError::EncodeError(
                    "Python dispatcher.dispatch must return bytes".to_string(),
                ))
            }
        })
    }
}

#[async_trait::async_trait]
impl actr_framework::MessageDispatcher for PyDispatcher {
    type Workload = PyWorkloadWrapper;

    async fn dispatch<C: FrameworkContext>(
        workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        let runtime_ctx = downcast_ctx(ctx)?;

        let payload = envelope
            .payload
            .as_ref()
            .map(|b| b.to_vec())
            .unwrap_or_default();
        let route_key = envelope.route_key.clone();

        let dispatcher_obj = workload.get_dispatcher().ok_or_else(|| {
            ProtocolError::TransportError(format!(
                "Workload does not provide a dispatcher. Please implement get_dispatcher() method. route_key: {}",
                route_key
            ))
        })?;

        PyDispatcher::dispatch_with_dispatcher(
            workload,
            dispatcher_obj,
            runtime_ctx,
            route_key,
            payload,
        )
        .await
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Workload trait implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[async_trait::async_trait]
impl Workload for PyWorkloadWrapper {
    type Dispatcher = PyDispatcher;

    // ── Lifecycle ────────────────────────────────────────

    async fn on_start<C: FrameworkContext>(&self, ctx: &C) -> ActorResult<()> {
        py_hook_ctx!(self, "on_start", ctx)
    }

    async fn on_ready<C: FrameworkContext>(&self, ctx: &C) -> ActorResult<()> {
        py_hook_ctx!(self, "on_ready", ctx)
    }

    async fn on_stop<C: FrameworkContext>(&self, ctx: &C) -> ActorResult<()> {
        py_hook_ctx!(self, "on_stop", ctx)
    }

    // Manual impl: extra `error: String` param doesn't fit py_hook_ctx! macro
    async fn on_error<C: FrameworkContext>(&self, ctx: &C, error: String) -> ActorResult<()> {
        let runtime_ctx = downcast_ctx(ctx)?;
        if !self.has_method("on_error")? {
            return Ok(());
        }
        let ctx_obj = self.wrap_context(runtime_ctx)?;
        let event_loop = self.require_event_loop()?;

        Python::attach(|py| -> PyResult<()> {
            let obj = self.py_obj.bind(py);
            let coro = obj.call_method1("on_error", (ctx_obj.clone_ref(py), error.clone()))?;
            let asyncio = py.import("asyncio")?;
            let future = asyncio
                .getattr("run_coroutine_threadsafe")?
                .call1((coro, event_loop.bind(py)))?;
            future.getattr("result")?.call0()?;
            Ok(())
        })
        .map_err(|e| ProtocolError::TransportError(format!("Python on_error failed: {e}")))?;
        Ok(())
    }

    // ── Signaling ───────────────────────────────────────

    async fn on_signaling_connect_start<C: FrameworkContext>(
        &self,
        ctx: Option<&C>,
    ) -> ActorResult<()> {
        py_hook_optional_ctx!(self, "on_signaling_connect_start", ctx)
    }

    async fn on_signaling_connected<C: FrameworkContext>(
        &self,
        ctx: Option<&C>,
    ) -> ActorResult<()> {
        py_hook_optional_ctx!(self, "on_signaling_connected", ctx)
    }

    async fn on_signaling_disconnected<C: FrameworkContext>(&self, ctx: &C) -> ActorResult<()> {
        py_hook_ctx!(self, "on_signaling_disconnected", ctx)
    }

    // ── WebSocket ───────────────────────────────────────

    async fn on_websocket_connect_start<C: FrameworkContext>(
        &self,
        ctx: &C,
        dest: &ActrId,
    ) -> ActorResult<()> {
        py_hook_ctx_dest!(self, "on_websocket_connect_start", ctx, dest)
    }

    async fn on_websocket_connected<C: FrameworkContext>(
        &self,
        ctx: &C,
        dest: &ActrId,
    ) -> ActorResult<()> {
        py_hook_ctx_dest!(self, "on_websocket_connected", ctx, dest)
    }

    async fn on_websocket_disconnected<C: FrameworkContext>(
        &self,
        ctx: &C,
        dest: &ActrId,
    ) -> ActorResult<()> {
        py_hook_ctx_dest!(self, "on_websocket_disconnected", ctx, dest)
    }

    // ── WebRTC ──────────────────────────────────────────

    async fn on_webrtc_connect_start<C: FrameworkContext>(
        &self,
        ctx: &C,
        dest: &ActrId,
    ) -> ActorResult<()> {
        py_hook_ctx_dest!(self, "on_webrtc_connect_start", ctx, dest)
    }

    // Manual impl: extra `relayed: bool` param doesn't fit py_hook_ctx_dest! macro
    async fn on_webrtc_connected<C: FrameworkContext>(
        &self,
        ctx: &C,
        dest: &ActrId,
        relayed: bool,
    ) -> ActorResult<()> {
        let runtime_ctx = downcast_ctx(ctx)?;
        if !self.has_method("on_webrtc_connected")? {
            return Ok(());
        }
        let ctx_obj = self.wrap_context(runtime_ctx)?;
        let dest = dest.clone();
        let event_loop = self.require_event_loop()?;

        Python::attach(|py| -> PyResult<()> {
            let obj = self.py_obj.bind(py);
            let dest_py = ActrIdPy::from_rust(dest.clone());
            let coro = obj.call_method1(
                "on_webrtc_connected",
                (ctx_obj.clone_ref(py), dest_py, relayed),
            )?;
            let asyncio = py.import("asyncio")?;
            let future = asyncio
                .getattr("run_coroutine_threadsafe")?
                .call1((coro, event_loop.bind(py)))?;
            future.getattr("result")?.call0()?;
            Ok(())
        })
        .map_err(|e| {
            ProtocolError::TransportError(format!("Python on_webrtc_connected failed: {e}"))
        })?;
        Ok(())
    }

    async fn on_webrtc_disconnected<C: FrameworkContext>(
        &self,
        ctx: &C,
        dest: &ActrId,
    ) -> ActorResult<()> {
        py_hook_ctx_dest!(self, "on_webrtc_disconnected", ctx, dest)
    }
}
