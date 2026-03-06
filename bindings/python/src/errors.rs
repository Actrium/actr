use actr_protocol::ActrError;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::PyErr;

create_exception!(actr_raw, ActrRuntimeError, PyException);
create_exception!(actr_raw, ActrTransportError, ActrRuntimeError);
create_exception!(actr_raw, ActrDecodeError, ActrRuntimeError);
create_exception!(actr_raw, ActrUnknownRoute, ActrRuntimeError);
create_exception!(actr_raw, ActrGateNotInitialized, ActrRuntimeError);

pub fn map_protocol_error(err: ActrError) -> PyErr {
    match err {
        ActrError::Unavailable(msg) => ActrTransportError::new_err(msg),
        ActrError::TimedOut => ActrTransportError::new_err("operation timed out"),
        ActrError::DecodeFailure(msg) => ActrDecodeError::new_err(msg),
        ActrError::UnknownRoute(route_key) => ActrUnknownRoute::new_err(route_key),
        other => ActrRuntimeError::new_err(other.to_string()),
    }
}
