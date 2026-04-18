#![allow(unsafe_op_in_unsafe_fn)]

use pyo3::prelude::*;

mod errors;
mod observability;
mod runtime;
mod types;

pub use errors::{
    ActrDecodeError, ActrGateNotInitialized, ActrRuntimeError, ActrTransportError, ActrUnknownRoute,
};

use runtime::{ActrNode, ActrRef, Context};
pub use types::{ActrId, ActrType, DataStream, Dest, PayloadType};

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
    m.add_class::<Dest>()?;
    m.add_class::<ActrId>()?;
    m.add_class::<ActrType>()?;
    m.add_class::<DataStream>()?;
    m.add_class::<ActrNode>()?;
    m.add_class::<ActrRef>()?;
    m.add_class::<Context>()?;

    Ok(())
}
