//! Exception hierarchy mirroring `actr_protocol::ActrError`.
//!
//! The tree is rooted at `ActrError` (with `ActrRuntimeError` kept as an
//! alias for backward compatibility) and splits into four fault-domain
//! intermediates (`Transient` / `Client` / `Corrupt` / `Internal`) whose
//! concrete subclasses map 1:1 onto the 10 core variants. A separate
//! `ActrGateNotInitializedError` captures pre-protocol binding state and
//! lives outside the fault-domain tree.
//!
//! Every exception exposes a `.kind` attribute (`"Transient"` / `"Client"` /
//! `"Corrupt"` / `"Internal"`) so downstream policy code can branch on
//! classification without knowing the concrete subclass.

use actr_protocol::{ActrError, Classify, ErrorKind};
use pyo3::Python;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::PyErr;
use pyo3::types::PyAnyMethods;

// ── Base + fault-domain intermediates ─────────────────────────────────────

create_exception!(actr_raw, ActrBaseError, PyException);

// Backwards-compatible alias: old code caught `ActrRuntimeError`, and we
// want that to keep working as the catch-all for every ACTR exception.
create_exception!(actr_raw, ActrRuntimeError, ActrBaseError);

create_exception!(actr_raw, ActrTransientError, ActrRuntimeError);
create_exception!(actr_raw, ActrClientError, ActrRuntimeError);
create_exception!(actr_raw, ActrCorruptError, ActrRuntimeError);
create_exception!(actr_raw, ActrInternalError, ActrRuntimeError);

// ── Concrete leaves (mirror the 10 core variants) ─────────────────────────

// Transient
create_exception!(actr_raw, ActrUnavailableError, ActrTransientError);
create_exception!(actr_raw, ActrTimedOutError, ActrTransientError);

// Client
create_exception!(actr_raw, ActrNotFoundError, ActrClientError);
create_exception!(actr_raw, ActrPermissionDeniedError, ActrClientError);
create_exception!(actr_raw, ActrInvalidArgumentError, ActrClientError);
create_exception!(actr_raw, ActrUnknownRouteError, ActrClientError);
create_exception!(actr_raw, ActrDependencyNotFoundError, ActrClientError);

// Corrupt
create_exception!(actr_raw, ActrDecodeFailureError, ActrCorruptError);

// Internal
create_exception!(actr_raw, ActrNotImplementedError, ActrInternalError);
create_exception!(actr_raw, ActrInternalFrameworkError, ActrInternalError);

// ── Legacy alias kept for back-compat with 0.2.x consumers ────────────────
// `ActrTransportError` / `ActrDecodeError` / `ActrUnknownRoute` were the
// only named leaves in 0.2.x; preserve the identifiers so old `except`
// clauses keep compiling — they now alias the new concrete subclasses.
pub type ActrTransportError = ActrUnavailableError;
pub type ActrDecodeError = ActrDecodeFailureError;
pub type ActrUnknownRoute = ActrUnknownRouteError;

// ── Python-local (pre-protocol) ───────────────────────────────────────────

create_exception!(actr_raw, ActrGateNotInitializedError, ActrBaseError);
// 0.2.x name kept so legacy `except ActrGateNotInitialized` still catches.
pub type ActrGateNotInitialized = ActrGateNotInitializedError;

// ── Mapping ───────────────────────────────────────────────────────────────

fn kind_str(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Transient => "Transient",
        ErrorKind::Client => "Client",
        ErrorKind::Internal => "Internal",
        ErrorKind::Corrupt => "Corrupt",
    }
}

/// Attach `kind` + `code` attributes to a freshly-constructed `PyErr`.
///
/// These two attributes let Python code write policy against the fault
/// domain (`e.kind == "Transient"`) or the concrete variant
/// (`e.code == "DependencyNotFound"`) without having to pattern-match on
/// subclass — useful for logging / metrics pipelines.
fn attach_metadata(err: PyErr, kind: ErrorKind, code: &str) -> PyErr {
    Python::attach(|py| {
        let value = err.value(py);
        let _ = value.setattr("kind", kind_str(kind));
        let _ = value.setattr("code", code);
    });
    err
}

/// Map a protocol-level error to the corresponding Python exception class.
///
/// Every core variant maps to a dedicated concrete subclass — there is no
/// "other → ActrRuntimeError" fallthrough, so downstream code can rely on
/// `except ActrNotFoundError` / `except ActrDependencyNotFoundError` /
/// etc. being raised exactly when the core framework raised the matching
/// variant.
pub fn map_protocol_error(err: ActrError) -> PyErr {
    let kind = err.kind();
    match err {
        ActrError::Unavailable(msg) => {
            attach_metadata(ActrUnavailableError::new_err(msg), kind, "Unavailable")
        }
        ActrError::TimedOut => attach_metadata(
            ActrTimedOutError::new_err("operation timed out"),
            kind,
            "TimedOut",
        ),
        ActrError::NotFound(msg) => {
            attach_metadata(ActrNotFoundError::new_err(msg), kind, "NotFound")
        }
        ActrError::PermissionDenied(msg) => attach_metadata(
            ActrPermissionDeniedError::new_err(msg),
            kind,
            "PermissionDenied",
        ),
        ActrError::InvalidArgument(msg) => attach_metadata(
            ActrInvalidArgumentError::new_err(msg),
            kind,
            "InvalidArgument",
        ),
        ActrError::UnknownRoute(msg) => {
            attach_metadata(ActrUnknownRouteError::new_err(msg), kind, "UnknownRoute")
        }
        ActrError::DependencyNotFound {
            service_name,
            message,
        } => {
            let display = format!("dependency '{service_name}' not found: {message}");
            let err = ActrDependencyNotFoundError::new_err(display);
            // Preserve the structured fields for Python callers that want
            // to read them off the exception instead of parsing the
            // stringified display form.
            Python::attach(|py| {
                let value = err.value(py);
                let _ = value.setattr("service_name", service_name.clone());
                let _ = value.setattr("message", message.clone());
            });
            attach_metadata(err, kind, "DependencyNotFound")
        }
        ActrError::DecodeFailure(msg) => {
            attach_metadata(ActrDecodeFailureError::new_err(msg), kind, "DecodeFailure")
        }
        ActrError::NotImplemented(msg) => attach_metadata(
            ActrNotImplementedError::new_err(msg),
            kind,
            "NotImplemented",
        ),
        ActrError::Internal(msg) => {
            attach_metadata(ActrInternalFrameworkError::new_err(msg), kind, "Internal")
        }
    }
}
