#![allow(unsafe_op_in_unsafe_fn)]

use pyo3::prelude::*;

mod errors;
mod observability;
mod runtime;
mod types;

pub use errors::{
    // Base + back-compat alias
    ActrBaseError,
    ActrClientError,
    ActrCorruptError,
    ActrDecodeError,
    ActrDecodeFailureError,
    ActrDependencyNotFoundError,
    ActrGateNotInitialized,
    ActrGateNotInitializedError,
    ActrInternalError,
    ActrInternalFrameworkError,
    ActrInvalidArgumentError,
    ActrNotFoundError,
    ActrNotImplementedError,
    ActrPermissionDeniedError,
    ActrRuntimeError,
    ActrTimedOutError,
    ActrTransientError,
    ActrTransportError,
    ActrUnavailableError,
    ActrUnknownRoute,
    ActrUnknownRouteError,
};

use runtime::{ActrNode, ActrRef, Context};
pub use types::{ActrId, ActrType, DataStream, Dest, PayloadType};

#[pymodule]
fn actr_raw(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Base + fault-domain intermediates
    m.add("ActrBaseError", _py.get_type::<ActrBaseError>())?;
    m.add("ActrRuntimeError", _py.get_type::<ActrRuntimeError>())?;
    m.add("ActrTransientError", _py.get_type::<ActrTransientError>())?;
    m.add("ActrClientError", _py.get_type::<ActrClientError>())?;
    m.add("ActrCorruptError", _py.get_type::<ActrCorruptError>())?;
    m.add("ActrInternalError", _py.get_type::<ActrInternalError>())?;

    // Concrete leaves (10 variants)
    m.add(
        "ActrUnavailableError",
        _py.get_type::<ActrUnavailableError>(),
    )?;
    m.add("ActrTimedOutError", _py.get_type::<ActrTimedOutError>())?;
    m.add("ActrNotFoundError", _py.get_type::<ActrNotFoundError>())?;
    m.add(
        "ActrPermissionDeniedError",
        _py.get_type::<ActrPermissionDeniedError>(),
    )?;
    m.add(
        "ActrInvalidArgumentError",
        _py.get_type::<ActrInvalidArgumentError>(),
    )?;
    m.add(
        "ActrUnknownRouteError",
        _py.get_type::<ActrUnknownRouteError>(),
    )?;
    m.add(
        "ActrDependencyNotFoundError",
        _py.get_type::<ActrDependencyNotFoundError>(),
    )?;
    m.add(
        "ActrDecodeFailureError",
        _py.get_type::<ActrDecodeFailureError>(),
    )?;
    m.add(
        "ActrNotImplementedError",
        _py.get_type::<ActrNotImplementedError>(),
    )?;
    m.add(
        "ActrInternalFrameworkError",
        _py.get_type::<ActrInternalFrameworkError>(),
    )?;

    // Python-local (pre-protocol)
    m.add(
        "ActrGateNotInitializedError",
        _py.get_type::<ActrGateNotInitializedError>(),
    )?;

    // Legacy 0.2.x aliases — kept so downstream `except` clauses keep working.
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
