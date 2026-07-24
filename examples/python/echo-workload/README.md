<!-- SPDX-License-Identifier: Apache-2.0 -->

# echo-workload (Python)

Generated Python EchoService workload built with `actr gen -l python`,
compiled to a `wasm32-wasip2` Component Model module, and packaged as a
signed `.actr`.

This example is the canonical Python workload example. It covers both the
typed Python codegen path and the `actr-workload` componentization path.
The generated dispatcher lives under `generated/`; `workload.py`
implements the business logic by returning an `EchoResponse` with
`reply="echo from python: " + req.message`.

## Regenerate

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
actr deps install
actr gen -l python --input protos --output generated
```

## Build

```bash
./build.sh
```

The script:

1. Creates `.venv/`.
2. Installs `../../../bindings/python/actr-workload[build]`, which pins
   `componentize-py==0.25.0`.
3. Runs `actr-workload bindings bindings --world-module actr_workload_bindings`.
4. Runs `actr-workload componentize workload --bindings-dir bindings`,
   with the local `actr-workload/src` directory on the componentizer
   Python path.
5. Runs `wasm-tools component wit` against the output and checks that
   the `actr:workload` interfaces appear in the metadata.

The output component is:

```text
dist/generated-echo-python-0.2.0-wasm32-wasip2.wasm
```

## Packaging

```bash
./build.sh package
```

The package output is:

```text
dist/acme-EchoService-0.2.0-wasm32-wasip2.actr
```

## Files

- `protos/local/echo.proto` — EchoService schema.
- `workload.py` — generated dispatcher-backed handler implementation.
- `requirements.txt` — local editable install of `actr-workload[build]` and protobuf.
- `build.sh` — regenerate, componentize, verify, and package flow.
- `manifest.toml` — actr packaging metadata.

## License

Apache-2.0 — see workspace [LICENSE](../../../LICENSE).
