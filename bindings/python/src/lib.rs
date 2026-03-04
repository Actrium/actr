#![allow(unsafe_op_in_unsafe_fn)]

use pyo3::prelude::*;

mod errors;
mod observability;
mod runtime;
mod types;
mod workload;

pub use errors::{
    ActrDecodeError, ActrGateNotInitialized, ActrRuntimeError, ActrTransportError, ActrUnknownRoute,
};

use runtime::{ActrNodePy, ActrRefPy, ActrSystemPy, ContextPy};
pub use types::{ActrIdPy, ActrTypePy, DataStreamPy, DestPy, PayloadType};

#[pymodule]
fn actr_raw(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("ActrRuntimeError", _py.get_type::<ActrRuntimeError>())?;
    m.add("ActrTransportError", _py.get_type::<ActrTransportError>())?;
    m.add("ActrDecodeError", _py.get_type::<ActrDecodeError>())?;
    m.add("ActrUnknownRoute", _py.get_type::<ActrUnknownRoute>())?;
    m.add(
        "ActrGateNotInitialized",
        _py.get_type::<ActrGateNotInitialized>(),
    )?;
    m.add_class::<PayloadType>()?;
    m.add_class::<DestPy>()?;
    m.add_class::<ActrIdPy>()?;
    m.add_class::<ActrTypePy>()?;
    m.add_class::<DataStreamPy>()?;
    m.add_class::<ActrSystemPy>()?;
    m.add_class::<ActrNodePy>()?;
    m.add_class::<ActrRefPy>()?;
    m.add_class::<ContextPy>()?;

    Ok(())
}
