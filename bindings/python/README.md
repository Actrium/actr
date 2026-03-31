# Actr Python SDK (`actr` + `actr_raw`)

The Python binding is now `package-first`. Source-defined local workloads were removed. The current Python API creates client-only nodes from `manifest.toml` and auto-loads the sibling `actr.toml`, then uses discovery plus explicit remote calls.

## Quick Start

```python
from actr import ActrNode, ActrType, Dest

async def main() -> None:
    node = await ActrNode.from_toml("manifest.toml")
    ref = await node.start()

    targets = await ref.discover(ActrType("actrium", "EchoService", "0.2.1-beta"), 1)
    if not targets:
        raise RuntimeError("No EchoService target discovered")

    response_bytes = await ref.call(
        Dest.actor(targets[0]),
        "echo.EchoService.Echo",
        request_proto,
    )

    await ref.wait_for_shutdown()
```

## API

- `ActrNode.from_toml(path)` creates a client-only node from `manifest.toml`
  and auto-loads `actr.toml` from the same directory.
- `ActrRef.discover(actr_type, count=1)` discovers remote actors.
- `ActrRef.call(target, route_key, request, timeout_ms=30000, payload_type=...)` sends a remote RPC.
- `ActrRef.tell(target, route_key, message, payload_type=...)` sends a one-way remote message.

## Current Scope

- Supported: client-only nodes, discovery, remote RPC, shutdown.
- Removed: source-defined Python workloads, `WorkloadBase`, `system.attach(...)`, local-service hosting from Python.
- For service hosting, build a verified `.actr` package and run it with Rust `Hyper.attach_package(...)`.

## Build

```bash
maturin develop --release
```
