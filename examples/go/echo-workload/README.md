<!-- SPDX-License-Identifier: Apache-2.0 -->

# echo-workload (Go)

Minimal actr workload authored in Go, compiled to `wasm32-wasip2`
Component Model via **TinyGo** + **wit-bindgen-go**, packaged as a
signed `.actr`.

This example proves that the actr workload contract (`actr-workload-guest`
world in `core/framework/wit/actr-workload.wit`) can host non-Rust guests
end-to-end.

## What it does

- Implements `dispatch(envelope) -> result<list<u8>, actr-error>` —
  echoes the inbound payload prefixed with `"echo: "` (raw bytes, **not**
  protobuf-decoded; cf. the Rust echo example which round-trips
  `EchoRequest`/`EchoResponse` via prost).
- Implements `on-start() -> result<_, actr-error>` returning `Ok(())`.
- The other 14 observation hooks are left as nil exports
  (wit-bindgen-go renders absent fields on the `Exports` struct as
  no-ops; the host treats absent observation hooks as no-ops too).

## Toolchain requirements

| Tool             | Version  | Purpose                                  |
|------------------|----------|------------------------------------------|
| `tinygo`         | >= 0.34  | wasip2 target, Component linker          |
| `wit-bindgen-go` | >= 0.6   | WIT -> Go bindings generator             |
| `wasm-tools`     | >= 1.219 | Component metadata verification          |
| `go`             | >= 1.23  | TinyGo dispatches to system Go for deps  |

Install hints (versions are illustrative, check upstream for current):

```bash
# TinyGo: see https://tinygo.org/getting-started/install/
# wit-bindgen-go:
go install go.bytecodealliance.org/cmd/wit-bindgen-go@latest
# wasm-tools:
cargo install wasm-tools
```

## Build

```bash
./build.sh           # generate + compile + verify world
./build.sh package   # additionally run `actr build --no-compile` to make .actr
```

The script:

1. Runs `wit-bindgen-go generate --world actr-workload-guest --out gen
   ../../../core/framework/wit/actr-workload.wit` to produce Go bindings
   under `gen/`.
2. Runs `go mod tidy` to resolve `go.bytecodealliance.org/cm`.
3. Runs `tinygo build -target=wasip2 -o dist/echo-go-...wasm ./...`.
4. Runs `wasm-tools component wit dist/echo-go-...wasm` and asserts the
   world `actr-workload-guest` appears in the metadata.

The output is a **Component Model** binary loadable by the actr host
through `Component::from_binary` (Phase 1).

## Packaging

`manifest.toml` declares the binary at `dist/echo-go-0.1.0-wasm32-wasip2.wasm`
with target `wasm32-wasip2` (which `actr_pack` resolves to
`BinaryKind::Component`). Because compilation is driven by TinyGo and
not Cargo, the manifest intentionally omits `[build]` — pack with:

```bash
actr build --no-compile -m manifest.toml
```

## Build verification status

This example was authored against the WIT contract at
`core/framework/wit/actr-workload.wit` (committed as part of the actr
Phase 1 Component Model rewrite). **Build was not run on the originating
host because TinyGo and wit-bindgen-go are not installed there.** The
following are unverified:

- The exact `gen/` package layout produced by `wit-bindgen-go` for
  `package actr:workload@0.1.0`. The `main.go` import paths
  (`echo-workload/gen/actr/workload/{types,workload}`) follow the
  generator's documented WIT-package -> Go-package convention; minor
  capitalization or sub-path adjustments may be needed after the first
  generation run.
- The exact `cm.Result[...]` type parameters for the WIT
  `result<list<u8>, actr-error>` shape. wit-bindgen-go has stabilised
  this shape across the 0.5 -> 0.6 series, but the type ordering should
  be cross-checked against the generated `workload.wit.go`.
- The `tinygo build -wit-package / -wit-world` flags assume TinyGo
  honours the wit-bindgen-go-generated cabi metadata. If TinyGo's
  built-in component linker disagrees, fall back to:

    ```bash
    tinygo build -target=wasip2 -o echo.core.wasm ./...
    wasm-tools component embed --world actr-workload-guest \
        ../../../core/framework/wit/actr-workload.wit echo.core.wasm \
        -o echo.embed.wasm
    wasm-tools component new echo.embed.wasm \
        -o dist/echo-go-0.1.0-wasm32-wasip2.wasm
    ```

A future `ci-go.yml` should pin the toolchain matrix and run the full
build chain. Until then, treat this example as **source-complete but
build-unverified** on the upstream CI.

## Files

- `main.go` — the workload (init() wires hooks; dispatch + on-start)
- `go.mod` — Go module + cm dependency pin
- `build.sh` — generate / build / verify pipeline
- `manifest.toml` — actr packaging metadata
- `.gitignore` — ignore `gen/`, `dist/`, `*.wasm`, `*.actr`

## License

Apache-2.0 — see workspace [LICENSE](../../../LICENSE).
