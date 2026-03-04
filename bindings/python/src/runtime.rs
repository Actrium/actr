use actr_config::ConfigParser;
use actr_framework::{Bytes, Context};
use actr_protocol::{PayloadType as RpPayloadType, ProtocolError};
use actr_runtime::context::RuntimeContext;
use actr_runtime::prelude::{ActrNode, ActrRef, ActrSystem};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::errors::map_protocol_error;
use crate::observability::ensure_observability_initialized;
use crate::types::{DataStreamPy, DestPy, PayloadType};
use crate::workload::PyWorkloadWrapper;
use crate::{ActrIdPy, ActrTypePy};

type WrappedWorkload = PyWorkloadWrapper;
type WrappedNode = ActrNode<WrappedWorkload>;
type WrappedRef = ActrRef<WrappedWorkload>;

#[pyclass(name = "ActrSystem")]
pub struct ActrSystemPy {
    pub(crate) inner: Option<ActrSystem>,
}

#[pymethods]
impl ActrSystemPy {
    #[staticmethod]
    fn from_toml<'py>(py: Python<'py>, path: String) -> PyResult<Bound<'py, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            eprintln!("[py] Loading config from: {}", path);
            let config =
                ConfigParser::from_file(&path).map_err(|e| PyValueError::new_err(e.to_string()))?;
            ensure_observability_initialized(Some(config.observability.clone()));
            eprintln!("[py] Config loaded, creating ActrSystem...");
            let system = ActrSystem::new(config).await.map_err(map_protocol_error)?;
            eprintln!("[py] ActrSystem created successfully");
            Python::attach(|py| {
                Py::new(
                    py,
                    ActrSystemPy {
                        inner: Some(system),
                    },
                )
                .map(Py::into_any)
            })
        })
    }

    fn attach(&mut self, workload: Py<PyAny>) -> PyResult<ActrNodePy> {
        eprintln!("[py] attach: taking system...");
        let system = self.inner.take().ok_or_else(|| {
            PyRuntimeError::new_err("ActrSystem already consumed (attach called twice)")
        })?;

        let event_loop = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let asyncio = py.import("asyncio")?;
            let get_event_loop = asyncio.getattr("get_event_loop")?;
            let loop_obj = get_event_loop.call0()?;
            Ok(loop_obj.extract::<Py<PyAny>>()?)
        })
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to get event loop: {e}")))?;

        eprintln!("[py] attach: creating PyWorkloadWrapper...");
        let mut py_workload = PyWorkloadWrapper::new(workload)?;
        py_workload.set_event_loop(event_loop);
        eprintln!("[py] attach: calling system.attach...");
        let node = system.attach(py_workload);
        eprintln!("[py] attach: system.attach completed");
        Ok(ActrNodePy { inner: Some(node) })
    }
}

#[pyclass(name = "ActrNode")]
pub struct ActrNodePy {
    pub(crate) inner: Option<WrappedNode>,
}

#[pymethods]
impl ActrNodePy {
    fn start<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let node = self.inner.take().ok_or_else(|| {
            PyRuntimeError::new_err("ActrNode already consumed (start called twice)")
        })?;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            eprintln!("[py] ActrNode.start: calling node.start()...");
            let actr_ref = node.start().await.map_err(map_protocol_error)?;
            eprintln!("[py] ActrNode.start: node.start() completed");
            Python::attach(|py| {
                Py::new(
                    py,
                    ActrRefPy {
                        inner: Some(actr_ref),
                    },
                )
                .map(Py::into_any)
            })
        })
    }

    fn try_start<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let node = self.inner.take().ok_or_else(|| {
            PyRuntimeError::new_err("ActrNode already consumed (try_start called twice)")
        })?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let actr_ref = node.start().await.map_err(map_protocol_error)?;
            Python::attach(|py| {
                Py::new(
                    py,
                    ActrRefPy {
                        inner: Some(actr_ref),
                    },
                )
                .map(Py::into_any)
            })
        })
    }
}

#[pyclass(name = "ActrRef")]
pub struct ActrRefPy {
    pub(crate) inner: Option<WrappedRef>,
}

#[pymethods]
impl ActrRefPy {
    fn actor_id(&self) -> PyResult<ActrIdPy> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("ActrRef has been consumed"))?;
        Ok(ActrIdPy::from_rust(inner.actor_id().clone()))
    }

    fn shutdown(&self) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("ActrRef has been consumed"))?;
        inner.shutdown();
        Ok(())
    }

    fn wait_for_shutdown<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("ActrRef has been consumed"))?
            .clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.wait_for_shutdown().await;
            Ok(Python::attach(|py| py.None()))
        })
    }

    fn wait_for_ctrl_c_and_shutdown<'py>(
        &mut self,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self
            .inner
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("ActrRef already consumed"))?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .wait_for_ctrl_c_and_shutdown()
                .await
                .map_err(map_protocol_error)?;
            Ok(Python::attach(|py| py.None()))
        })
    }

    #[pyo3(signature = (route_key, request, timeout_ms=30000, payload_type=PayloadType::RpcReliable))]
    fn call<'py>(
        &self,
        py: Python<'py>,
        route_key: String,
        request: &[u8],
        timeout_ms: i64,
        payload_type: PayloadType,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("ActrRef has been consumed"))?
            .clone();
        let request_bytes = Bytes::from(request.to_vec());

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response_bytes = inner
                .call_raw(route_key, request_bytes, timeout_ms, payload_type.to_rust())
                .await
                .map_err(map_protocol_error)?;

            Python::attach(|py| Ok(PyBytes::new(py, &response_bytes).into_any().into()))
                .map(Py::into_any)
        })
    }

    #[pyo3(signature = (route_key, message, payload_type=PayloadType::RpcReliable))]
    fn tell<'py>(
        &self,
        py: Python<'py>,
        route_key: String,
        message: &[u8],
        payload_type: PayloadType,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("ActrRef has been consumed"))?
            .clone();
        let message_bytes = Bytes::from(message.to_vec());

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .tell_raw(route_key, message_bytes, payload_type.to_rust())
                .await
                .map_err(map_protocol_error)?;
            Ok(Python::attach(|py| py.None()))
        })
    }
}

#[pyclass(name = "Context")]
pub struct ContextPy {
    pub(crate) inner: RuntimeContext,
}

#[pymethods]
impl ContextPy {
    fn self_id(&self) -> PyResult<ActrIdPy> {
        Ok(ActrIdPy::from_rust(self.inner.self_id().clone()))
    }

    fn caller_id(&self) -> PyResult<Option<ActrIdPy>> {
        if let Some(id) = self.inner.caller_id() {
            Ok(Some(ActrIdPy::from_rust(id.clone())))
        } else {
            Ok(None)
        }
    }

    fn request_id(&self) -> String {
        self.inner.request_id().to_string()
    }

    fn discover_route_candidate<'py>(
        &self,
        py: Python<'py>,
        actr_type: ActrTypePy,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ctx = self.inner.clone();
        let target_type = actr_type.inner().clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let id = ctx
                .discover_route_candidate(&target_type)
                .await
                .map_err(map_protocol_error)?;
            Python::attach(|py| Ok(Py::new(py, ActrIdPy::from_rust(id))?.into_any()))
        })
    }

    #[pyo3(signature = (target, route_key, request, timeout_ms=30000, payload_type=PayloadType::RpcReliable))]
    fn call_raw<'py>(
        &self,
        py: Python<'py>,
        target: &DestPy,
        route_key: String,
        request: &[u8],
        timeout_ms: i64,
        payload_type: PayloadType,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ctx = self.inner.clone();
        let target_dest = target.inner().clone();
        let request_bytes = Bytes::from(request.to_vec());
        let payload_type_rust = payload_type.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bytes = ctx
                .call_raw(
                    &target_dest,
                    route_key,
                    payload_type_rust,
                    request_bytes,
                    timeout_ms,
                )
                .await
                .map_err(map_protocol_error)?;
            Python::attach(|py| Ok(PyBytes::new(py, &bytes).into_any().into())).map(Py::into_any)
        })
    }

    #[pyo3(signature = (target, route_key, message, payload_type=PayloadType::RpcReliable))]
    fn tell_raw<'py>(
        &self,
        py: Python<'py>,
        target: &DestPy,
        route_key: String,
        message: &[u8],
        payload_type: PayloadType,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ctx = self.inner.clone();
        let target_dest = target.inner().clone();
        let message_bytes = Bytes::from(message.to_vec());
        let payload_type_rust = payload_type.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            ctx.tell_raw(&target_dest, route_key, payload_type_rust, message_bytes)
                .await
                .map_err(map_protocol_error)?;
            Ok(Python::attach(|py| py.None()))
        })
    }

    #[pyo3(signature = (stream_id, callback))]
    fn register_stream<'py>(
        &self,
        py: Python<'py>,
        stream_id: String,
        callback: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ctx = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let callback_py = Python::attach(|py| callback.clone_ref(py));
            ctx.register_stream(stream_id.clone(), move |chunk, sender_id| {
                let callback_clone = Python::attach(|py| callback_py.clone_ref(py));
                let stream_id_debug = stream_id.clone();
                Box::pin(async move {
                    tracing::info!(
                        "[py] Stream callback invoked: stream_id={}, seq={}, size={} bytes, sender={:?}",
                        stream_id_debug,
                        chunk.sequence,
                        chunk.payload.len(),
                        sender_id
                    );

                    let (py_ds, py_sender_id) = Python::attach(|py| -> PyResult<(Py<PyAny>, Py<PyAny>)> {
                        let ds: Py<PyAny> = Py::new(py, DataStreamPy::from_rust(chunk.clone()))?.into();
                        let sender_id_rust = sender_id.clone();
                        let sid: Py<PyAny> = Py::new(py, ActrIdPy::from_rust(sender_id_rust))?.into();
                        Ok((ds, sid))
                    })
                    .map_err(|e| ProtocolError::TransportError(format!("Failed to convert to Python objects: {e}")))?;

                    let result = tokio::task::spawn_blocking(move || {
                        Python::attach(|py| -> PyResult<Py<PyAny>> {
                            let callback_obj = callback_clone.bind(py);
                            let py_ds_obj = py_ds.bind(py);
                            let py_sender_id_obj = py_sender_id.bind(py);
                            let coro = callback_obj.call1((py_ds_obj, py_sender_id_obj))?;
                            let asyncio = py.import("asyncio")?;
                            let run_func = asyncio.getattr("run")?;
                            let result = run_func.call1((coro,))?;
                            Ok(result.into_any().into())
                        })
                    })
                    .await
                    .map_err(|e| ProtocolError::TransportError(format!("Task join failed: {e}")))?;

                    match result {
                        Ok(_) => {
                            tracing::debug!("[py] Stream callback completed successfully: stream_id={}", stream_id_debug);
                            Ok(())
                        }
                        Err(e) => {
                            tracing::error!("[py] Stream callback error: stream_id={}, error={:?}", stream_id_debug, e);
                            Err(ProtocolError::TransportError(format!("Python callback error: {e}")))
                        }
                    }
                })
            })
            .await
            .map_err(map_protocol_error)?;

            Ok(Python::attach(|py| py.None()))
        })
    }

    fn unregister_stream<'py>(
        &self,
        py: Python<'py>,
        stream_id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ctx = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            ctx.unregister_stream(&stream_id)
                .await
                .map_err(map_protocol_error)?;
            Ok(Python::attach(|py| py.None()))
        })
    }

    fn send_data_stream<'py>(
        &self,
        py: Python<'py>,
        target: &DestPy,
        data_stream: &DataStreamPy,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ctx = self.inner.clone();
        let target_dest = target.inner().clone();
        let chunk = data_stream.inner().clone();
        let stream_id = chunk.stream_id.clone();
        let sequence = chunk.sequence;

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            tracing::info!(
                "[py] send_data_stream: target={:?}, stream_id={}, sequence={}, payload_size={} bytes",
                target_dest,
                stream_id,
                sequence,
                chunk.payload.len()
            );

            ctx.send_data_stream_with_type(&target_dest, RpPayloadType::StreamReliable, chunk)
                .await
                .map_err(|e| {
                    tracing::error!(
                        "[py] send_data_stream FAILED: target={:?}, stream_id={}, sequence={}, error={:?}",
                        target_dest,
                        stream_id,
                        sequence,
                        e
                    );
                    e
                })
                .map_err(map_protocol_error)?;

            tracing::info!(
                "[py] send_data_stream SUCCESS: target={:?}, stream_id={}, sequence={}",
                target_dest,
                stream_id,
                sequence
            );

            Ok(Python::attach(|py| py.None()))
        })
    }
}
