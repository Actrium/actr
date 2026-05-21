<!-- SPDX-License-Identifier: Apache-2.0 -->

# echo-workload (Python)

Minimal actr workload authored in Python, compiled to `wasm32-wasip2`
Component Model via **componentize-py**, packaged as a signed `.actr`.

This example demonstrates the Python authoring path for the actr workload
contract (`actr-workload-guest` world in
[`core/framework/wit/actr-workload.wit`](../../../core/framework/wit/actr-workload.wit)).
The Python source, bindings generated from WIT, and a bundled CPython
WASM interpreter are packed into a single Component that the actr host
loads through `Component::from_binary`.

## Alpha toolchain warning

`componentize-py` is an **alpha** project from the Bytecode Alliance. As
of this commit:

- Import paths and generated class names in `bindings/` have shifted
  across 0.15 / 0.16 / 0.17. This example pins `componentize-py==0.17.2`
  (see `requirements.txt`).
- First-run builds download a prebuilt CPython WASM interpreter; no
  internet = no build.
- The resulting Component is roughly **10 MB** because it embeds CPython
  plus a stdlib subset. For size-sensitive deployments, prefer the Go /
  C / Rust echo examples.
- Startup is slower than Rust/Go guests: CPython cold-boots inside wasm
  before the first `dispatch`. Acceptable for demo and server workloads,
  generally not for latency-critical paths.

## What it does

- Implements `dispatch(envelope) -> result<list<u8>, actr-error>` —
  echoes the inbound payload prefixed with `"echo: "` (raw bytes, not
  protobuf-decoded).
- Implements `on-start` / `on-ready` / `on-stop` / `on-error` as
  fallible no-ops returning `Ok(())`.
- Implements the twelve observation hooks (signaling / transport /
  credential / mailbox) as infallible no-ops.

## Toolchain requirements

| Tool              | Version     | Purpose                                          |
|-------------------|-------------|--------------------------------------------------|
| `python3`         | >= 3.10     | host interpreter that runs componentize-py       |
| `pip`             | >= 23       | install componentize-py + deps                   |
| `componentize-py` | 0.17.2      | WIT -> Python bindings + Component bundler       |
| `wasm-tools`      | >= 1.219    | Component metadata verification                  |

Install hints (Linux):

```bash
# componentize-py pulls in a precompiled CPython-WASM wheel
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt

# wasm-tools (from Rust cargo or prebuilt release)
cargo install wasm-tools
```

## Build

```bash
./build.sh              # venv + bindings + componentize + verify
./build.sh package      # additionally run `actr build --no-compile` to make .actr
```

The script:

1. Creates `.venv/` and installs `componentize-py==0.17.2`.
2. Runs `componentize-py bindings` to emit a Python package tree under
   `bindings/` mapping the WIT types and interfaces.
3. Runs `componentize-py componentize workload -o dist/echo-python-...wasm`
   which bundles `workload.py`, the generated bindings, and the CPython
   WASM interpreter into one Component.
4. Runs `wasm-tools component wit` against the output and asserts the
   `actr:workload` interfaces appear in the metadata.

## Build verification status

This example was authored on a host that does **not** have
`componentize-py` installed. The build pipeline was **not** executed
end-to-end — treat the example as source-complete but build-unverified.

Specifically, the following still need a machine with componentize-py:

- Exact module path produced by `componentize-py bindings` for
  `package actr:workload@0.1.0` + world `actr-workload-guest`. The
  `workload.py` source uses the 0.17.x documented mapping
  (`from actr_workload.exports import workload as workload_exports`),
  but minor adjustments may be needed once the tree is generated.
- The on-disk layout of the generated `Workload` base class and whether
  componentize-py expects our subclass to be named `Workload` (0.17.x)
  or `<InterfaceName>` with a specific module path.
- End-to-end round-trip: launching the packed `.actr` via `actr run -c
  actr.toml` and observing the echo response.

A future `ci-python.yml` would pin the componentize-py version and run
the build chain. Until then, this example carries the same
"source-complete, build-unverified" status as the Go and C examples on
hosts without their respective toolchains installed.

## Packaging

`manifest.toml` declares the binary at
`dist/echo-python-0.1.0-wasm32-wasip2.wasm` with target `wasm32-wasip2`
(which `actr_pack` resolves to `BinaryKind::Component`). Because
compilation is driven by componentize-py and not Cargo, the manifest
intentionally omits `[build]` — pack with:

```bash
actr build --no-compile -m manifest.toml
```

## Files

- `workload.py` — the workload class (dispatch + lifecycle + observations)
- `requirements.txt` — pin `componentize-py==0.17.2`
- `build.sh` — venv / bindings / componentize / verify pipeline
- `manifest.toml` — actr packaging metadata
- `.gitignore` — ignore `bindings/`, `dist/`, `.venv/`, `__pycache__/`

## License

Apache-2.0 — see workspace [LICENSE](../../../LICENSE).
