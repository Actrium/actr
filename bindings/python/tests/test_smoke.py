"""Smoke tests for the actr Python module load.

These tests verify that the maturin-built extension imports cleanly and
exposes the high-level public surface declared in ``actr/__init__.py``.
They do not exercise runtime behaviour — they only assert that the
public API surface stays intact across releases.
"""

import importlib

import pytest

actr = importlib.import_module("actr")


def test_module_imports():
    assert actr.__version__ is not None
    assert actr.__version__ != ""


@pytest.mark.parametrize(
    "name",
    [
        # High-level Pythonic API (root package re-exports).
        "ActrNode",
        "ActrRef",
        "Context",
        "ActrId",
        "ActrType",
        "Dest",
        "PayloadType",
        "DataStream",
        # Exception hierarchy mirrors actr_protocol::ActrError.
        "ActrBaseError",
        "ActrRuntimeError",
        "ActrTransientError",
        "ActrClientError",
        "ActrCorruptError",
        "ActrInternalError",
        "ActrUnavailableError",
        "ActrTimedOutError",
        "ActrNotFoundError",
        "ActrPermissionDeniedError",
        "ActrInvalidArgumentError",
        "ActrUnknownRouteError",
        "ActrDependencyNotFoundError",
        "ActrDecodeFailureError",
        "ActrNotImplementedError",
        "ActrInternalFrameworkError",
        "ActrGateNotInitializedError",
        # Legacy 0.2.x aliases — still re-exported.
        "ActrTransportError",
        "ActrDecodeError",
        "ActrUnknownRoute",
        "ActrGateNotInitialized",
        # Submodule access for advanced callers.
        "actr_raw",
    ],
)
def test_public_symbol_exposed(name):
    assert hasattr(actr, name), f"actr.{name} missing from public API"


def test_payload_type_values():
    assert actr.PayloadType.RpcReliable is not None
    assert actr.PayloadType.RpcSignal is not None
    assert actr.PayloadType.StreamReliable is not None
    assert actr.PayloadType.StreamLatencyFirst is not None
    assert actr.PayloadType.MediaRtp is not None


def test_actr_type_construct():
    t = actr.ActrType("acme", "Demo", "1.0.0")
    repr_str = repr(t)
    assert "acme" in repr_str
    assert "Demo" in repr_str
    assert "1.0.0" in repr_str
