# Actr Python SDK (`actr` + `actr_raw`)

Actr's Python SDK is a mixed Rust/Python package that ships a Pyo3-powered extension module (`actr_raw`) together with a Pythonic wrapper layer (`actr`). The Rust layer exposes the raw bindings, while the `actr` layer wraps those bindings into a high-level `ActrSystem` API (plus other high-level helpers) for application use.

## Layout
- `pyproject.toml` – Python packaging metadata for maturin/wheel builds
- `Cargo.toml` / `src/` – Rust extension implemented with Pyo3 (exports `actr_raw`)
- `python/` – Pure-Python helpers and examples (packaged as `actr`)

## Quick start
Requirements: Python 3.9+, Rust toolchain, `pip install maturin>=1.8`.

```bash
cd actr-python
maturin develop --release  # build and install into the current virtualenv
```

Then use the SDK (implement your own `Handler` + `WorkloadBase`):

```python
from actr import ActrSystem, WorkloadBase
from generated import my_service_pb2
from generated import my_service_actor

class MyHandler(my_service_actor.MyServiceHandler):
    async def echo(self, req: my_service_pb2.EchoRequest, ctx):
        return my_service_pb2.EchoResponse(message=f"Echo: {req.message}")

class MyWorkload(WorkloadBase):
    def __init__(self, handler: MyHandler):
        self.handler = handler
        super().__init__(my_service_actor.MyServiceDispatcher())

    async def on_start(self, ctx):
        pass

    async def on_stop(self, ctx):
        pass

system = await ActrSystem.from_toml("Actr.toml")
node = system.attach(MyWorkload(MyHandler()))
ref = await node.start()
await ref.wait_for_ctrl_c_and_shutdown()
```

## Building wheels
```bash
# Build a release wheel and sdist into ./target/wheels
maturin build --release
```

Artifacts produced by `maturin build` include the Rust extension (`actr_raw`) and the Python sources from `python/`, ready for publishing to an internal index or PyPI.

**Note:** 
- When using `pip install` directly on the source directory
- For development, use `maturin develop --release` which will install both `actr_raw` and `actr` packages (via editable install).

## Development notes
- The crate depends on the local Rust workspace crates under `../actr/crates/*`; keep those paths in sync when moving the SDK.
- Set `RUST_LOG=debug` while developing to surface runtime diagnostics from the binding layer.
- Python typing support is shipped via `python/actr/py.typed`.

---

# Architecture Documentation

This section provides detailed information about the `actr` Python package architecture, package structure, and implementation details.

## Package Structure

```
actr/
├── __init__.py          # Root package entry, exports all public APIs
├── actr_raw/            # Rust binding layer (low-level API, imports actr_raw)
└── workload.py          # WorkloadBase (developers implement their own Handler + Workload)
```

### Module Overview

#### `actr_raw/` - Rust Binding Layer

**Responsibility**: Directly import and export all types and methods from the Rust extension module.

**Contents**:
- Import all types from the Rust extension module (`actr_raw`)
- Provide a unified import interface

**Exports**:
- `ActrSystem`, `ActrNode`, `ActrRef`, `Context`
- `ActrId`, `ActrType`, `Dest`, `PayloadType`
- `DataStream` 
- All exception types: `ActrRuntimeError`, `ActrTransportError`, etc.

**Use Cases**:
- Advanced users who need direct access to Rust APIs
- Scenarios requiring fine-grained control

#### `__init__.py` - High-level Pythonic API

**Responsibility**: Provide Python-friendly high-level APIs that wrap the Rust bindings.

**Main Classes**:

```text
ActrSystem
- from_toml(path: str) -> ActrSystem
  # Build system from TOML config file.
- attach(workload: WorkloadBase) -> ActrNode
  # Attach a workload instance to the system.

ActrNode
- start() -> ActrRef
  # Start the node and return a reference.

ActrRef
- call(route_key: str, request, timeout_ms: int = 30000, payload_type: PayloadType = None) -> bytes
  # Call RPC and return response bytes.
- tell(route_key: str, message, payload_type: PayloadType = None) -> None
  # Fire-and-forget RPC.
- shutdown() -> None
  # Request actor shutdown.
- wait_for_shutdown() -> None
  # Await node shutdown.
- wait_for_ctrl_c_and_shutdown() -> None
  # Await Ctrl+C then shutdown.

Context
- call(target: Dest, route_key: str, request, timeout_ms: int = 30000, payload_type: PayloadType = None) -> bytes
  # Call RPC to target and return response bytes.
- tell(target: Dest, route_key: str, message, payload_type: PayloadType = None) -> None
  # Fire-and-forget RPC to target.
- discover(actr_type: ActrType) -> ActrId
  # Discover an actor by type.
- register_stream(stream_id: str, callback) -> None
  # Register stream callback: callback(data_stream: DataStream, sender_id: ActrId).
- unregister_stream(stream_id: str) -> None
  # Unregister stream callback.
- send_stream(target: Dest, data_stream: DataStream, payload_type: PayloadType = None) -> None
  # Send DataStream to target.
```

Handlers receive `Context` from the dispatcher; developers implement `Handler` + `WorkloadBase`.

**DataStream constructor**:
```python
from actr import DataStream

# From fields
stream = DataStream(stream_id="stream-1", sequence=1, payload=b"hello")
```

**Features**:
- **Auto serialization**: Accept protobuf request objects and automatically serialize to bytes
- **Manual response decode**: RPC responses are returned as `bytes` and should be deserialized via `ResponseType.FromString(response_bytes)`
- **Direct exception raising**: All methods directly raise exceptions, following Python conventions
- **Manual destination wrapping**: Wrap actor id protobuf objects with `Dest.actor(actor_id)` before calling `Context.call()`/`Context.tell()`/`Context.send_stream()`
- **Pythonic design**: Concise method names, reasonable parameters, following Python conventions

**Exports from `__init__.py`**:
- High-level APIs: `ActrSystem`, `ActrNode`, `ActrRef`, `Context`
- Types: `ActrId`, `ActrType`, `Dest`, `PayloadType`, `DataStream`
- Exceptions: `ActrRuntimeError`, `ActrTransportError`, etc.
- Rust binding: `actr_raw` module (for advanced usage)

---

## Design Philosophy

### 1. Layered Architecture

```
┌─────────────────────────────────────┐
│  High-level Pythonic API (ActrSystem, ActrRef, Context) │  ← Python-friendly, recommended
├─────────────────────────────────────┤
│  Rust Binding (actr_raw/)            │  ← Low-level, direct Rust mapping
└─────────────────────────────────────┘
```

### 2. Pythonic Design Principles

- **Direct exception raising**: All high-level API methods directly raise exceptions, following Python conventions
  - Use standard `try/except` for error handling
  - Exception types: `ActrRuntimeError`, `ActrTransportError`, `ActrDecodeError`, etc.
- **Auto serialization**: Accept protobuf request objects and automatically serialize to bytes
- **Manual response decode**: RPC responses are returned as `bytes` and should be deserialized via `ResponseType.FromString(response_bytes)`
- **Concise API**: Concise method names, reasonable parameters

### 3. Backward Compatibility

- Preserve direct access to Rust bindings (`actr_raw` module)

---

## API Layers

### Layer 1: High-level Pythonic API

**Features**: Python-friendly, automatic serialization handling

```python
from actr import ActrSystem, ActrRef, Context

# Use high-level API
system = await ActrSystem.from_toml("Actr.toml")
workload = MyWorkload()
node = system.attach(workload)
ref = await node.start()

# Call RPC (auto serialize, directly raises exceptions)
try:
    response_bytes = await ref.call("route.key", request_obj)
    response_obj = ResponseType.FromString(response_bytes)
except ActrRuntimeError as e:
    # Handle error
    pass
```

### Layer 2: Rust Binding (Advanced Users)

**Features**: Direct access to Rust APIs, maximum control

```python
from actr.actr_raw import ActrSystem, ActrRef

# Use Rust binding
system = await ActrSystem.from_toml("Actr.toml")
# Need manual serialization/deserialization of protobuf messages
```

---

## Usage Guide

### Quick Start (Recommended)

```python
from actr import ActrSystem, WorkloadBase, Context
from generated import my_service_pb2, my_service_actor

class MyHandler(my_service_actor.MyServiceHandler):
    async def echo(self, req: my_service_pb2.EchoRequest, ctx: Context):
        return my_service_pb2.EchoResponse(message=req.message)

class MyWorkload(WorkloadBase):
    def __init__(self, handler: MyHandler):
        self.handler = handler
        super().__init__(my_service_actor.MyServiceDispatcher())

    async def on_start(self, ctx: Context):
        pass

    async def on_stop(self, ctx: Context):
        pass

async def main():
    system = await ActrSystem.from_toml("Actr.toml")
    workload = MyWorkload(MyHandler())
    node = system.attach(workload)
    ref = await node.start()
```

### Using Rust Binding (Advanced Usage)

**Note**: Rust Binding now also directly raises exceptions, consistent with high-level API behavior.

```python
from actr.actr_raw import ActrSystem, ActrRef, ActrRuntimeError

async def main():
    try:
        system = await ActrSystem.from_toml("Actr.toml")
        workload = MyWorkload()
        node = system.attach(workload)
        ref = await node.start()  # Directly raises exception, no need to check is_ok()
    except ActrRuntimeError as e:
        # Handle error
        pass
```

**Note**: Rust Binding and high-level API now use the same exception handling mechanism. All methods directly raise exceptions, following Python conventions.

---

## Summary

### Recommended Usage

- **Daily Development**: Use high-level API (`ActrSystem`, `ActrRef`, `Context`) with your own Handler + Workload
- **Advanced Scenarios**: Use Rust Binding (`actr_raw` module)
