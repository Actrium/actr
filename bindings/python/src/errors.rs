use actr_protocol::{ActrError, ProtocolError};
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::PyErr;

create_exception!(actr_raw, ActrRuntimeError, PyException);
create_exception!(actr_raw, ActrTransportError, ActrRuntimeError);
create_exception!(actr_raw, ActrDecodeError, ActrRuntimeError);
create_exception!(actr_raw, ActrUnknownRoute, ActrRuntimeError);
create_exception!(actr_raw, ActrGateNotInitialized, ActrRuntimeError);

pub fn map_protocol_error(err: ProtocolError) -> PyErr {
    match err {
        ProtocolError::TransportError(msg) => ActrTransportError::new_err(msg),
        ProtocolError::DecodeError(msg)
        | ProtocolError::DeserializationError(msg)
        | ProtocolError::EncodeError(msg) => ActrDecodeError::new_err(msg),
        ProtocolError::UnknownRoute(msg) => ActrUnknownRoute::new_err(msg),
        ProtocolError::Actr(ActrError::UnknownRoute { route_key }) => {
            ActrUnknownRoute::new_err(route_key)
        }
        ProtocolError::Actr(ActrError::GateNotInitialized { message }) => {
            ActrGateNotInitialized::new_err(message)
        }
        other => ActrRuntimeError::new_err(other.to_string()),
    }
}
