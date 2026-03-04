use std::any::Any;

use actr_framework::{Bytes, Context as FrameworkContext, Workload};
use actr_protocol::{ActorResult, ProtocolError, RpcEnvelope};
use actr_runtime::context::RuntimeContext;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyString};

use crate::runtime::ContextPy;

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
}

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
        let any = ctx as &dyn Any;
        let runtime_ctx = any.downcast_ref::<RuntimeContext>().ok_or_else(|| {
            ProtocolError::TransportError("Context is not RuntimeContext".to_string())
        })?;

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

#[async_trait::async_trait]
impl Workload for PyWorkloadWrapper {
    type Dispatcher = PyDispatcher;

    async fn on_start<C: FrameworkContext>(&self, ctx: &C) -> ActorResult<()> {
        let any = ctx as &dyn Any;
        let runtime_ctx = any.downcast_ref::<RuntimeContext>().ok_or_else(|| {
            ProtocolError::TransportError("Context is not RuntimeContext".to_string())
        })?;

        let ctx_py = Python::attach(|py| self.make_context_py(py, runtime_ctx)).map_err(|e| {
            ProtocolError::TransportError(format!("Failed to create ContextPy: {e}"))
        })?;

        let ctx_obj = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let actr_module = py.import("actr")?;
            let ctx_class = actr_module.getattr("Context")?;
            let ctx_obj = ctx_class.call1((ctx_py.clone_ref(py),))?;
            Ok(ctx_obj.extract::<Py<PyAny>>()?)
        })
        .map_err(|e| ProtocolError::TransportError(format!("Failed to wrap Context: {e}")))?;

        let maybe = Python::attach(|py| {
            let obj = self.py_obj.bind(py);
            Ok(obj.hasattr("on_start")?)
        })
        .map_err(|e: PyErr| ProtocolError::TransportError(format!("Python hasattr failed: {e}")))?;

        if !maybe {
            return Ok(());
        }

        let fut = Python::attach(|py| -> PyResult<_> {
            let obj = self.py_obj.bind(py);
            let ctx_obj = ctx_obj.clone_ref(py);
            let coro = obj.call_method1("on_start", (ctx_obj,))?;
            pyo3_async_runtimes::tokio::into_future(coro)
        })
        .map_err(|e| ProtocolError::TransportError(format!("Python on_start call failed: {e}")))?;

        fut.await.map_err(|e| {
            ProtocolError::TransportError(format!("Python on_start await failed: {e}"))
        })?;
        Ok(())
    }

    async fn on_stop<C: FrameworkContext>(&self, ctx: &C) -> ActorResult<()> {
        let any = ctx as &dyn Any;
        let runtime_ctx = any.downcast_ref::<RuntimeContext>().ok_or_else(|| {
            ProtocolError::TransportError("Context is not RuntimeContext".to_string())
        })?;

        let ctx_py = Python::attach(|py| self.make_context_py(py, runtime_ctx)).map_err(|e| {
            ProtocolError::TransportError(format!("Failed to create ContextPy: {e}"))
        })?;

        let ctx_obj = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let actr_module = py.import("actr")?;
            let ctx_class = actr_module.getattr("Context")?;
            let ctx_obj = ctx_class.call1((ctx_py.clone_ref(py),))?;
            Ok(ctx_obj.extract::<Py<PyAny>>()?)
        })
        .map_err(|e| ProtocolError::TransportError(format!("Failed to wrap Context: {e}")))?;

        let maybe = Python::attach(|py| {
            let obj = self.py_obj.bind(py);
            Ok(obj.hasattr("on_stop")?)
        })
        .map_err(|e: PyErr| ProtocolError::TransportError(format!("Python hasattr failed: {e}")))?;

        if !maybe {
            return Ok(());
        }

        let fut = Python::attach(|py| -> PyResult<_> {
            let obj = self.py_obj.bind(py);
            let ctx_obj = ctx_obj.clone_ref(py);
            let coro = obj.call_method1("on_stop", (ctx_obj,))?;
            pyo3_async_runtimes::tokio::into_future(coro)
        })
        .map_err(|e| ProtocolError::TransportError(format!("Python on_stop call failed: {e}")))?;

        fut.await.map_err(|e| {
            ProtocolError::TransportError(format!("Python on_stop await failed: {e}"))
        })?;
        Ok(())
    }
}
