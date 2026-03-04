"""
Rust bindings for actr-runtime.

This module loads the compiled Pyo3 extension (`actr_raw`) and re-exports
its symbols. Import failures are surfaced as an ImportError instead of silently
falling back to stubs, so packaging/build issues are easier to diagnose.
"""

from __future__ import annotations

from importlib import import_module

try:
    _rust_module = import_module("actr_raw")
except ImportError as exc:
    raise ImportError(
        "actr_raw extension module not found. Build and install it with `maturin develop` "
        "or ensure the wheel is installed."
    ) from exc

ActrSystem = getattr(_rust_module, "ActrSystem")
ActrNode = getattr(_rust_module, "ActrNode")
ActrRef = getattr(_rust_module, "ActrRef")
Context = getattr(_rust_module, "Context")
ActrId = getattr(_rust_module, "ActrId")
ActrType = getattr(_rust_module, "ActrType")
Dest = getattr(_rust_module, "Dest")
PayloadType = getattr(_rust_module, "PayloadType")
DataStream = getattr(_rust_module, "DataStream")
ActrRuntimeError = getattr(_rust_module, "ActrRuntimeError")
ActrTransportError = getattr(_rust_module, "ActrTransportError")
ActrDecodeError = getattr(_rust_module, "ActrDecodeError")
ActrUnknownRoute = getattr(_rust_module, "ActrUnknownRoute")
ActrGateNotInitialized = getattr(_rust_module, "ActrGateNotInitialized")

__all__ = [
    "ActrSystem",
    "ActrNode",
    "ActrRef",
    "Context",
    "ActrId",
    "ActrType",
    "Dest",
    "PayloadType",
    "DataStream",
    "ActrRuntimeError",
    "ActrTransportError",
    "ActrDecodeError",
    "ActrUnknownRoute",
    "ActrGateNotInitialized",
]
