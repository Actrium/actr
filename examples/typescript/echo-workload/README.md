<!-- SPDX-License-Identifier: Apache-2.0 -->

# echo-workload (TypeScript)

Minimal actr **workload** authored in TypeScript, compiled to
`wasm32-wasip2` Component Model via
[`@bytecodealliance/jco componentize`](https://github.com/bytecodealliance/jco)
(which delegates to ComponentizeJS + StarlingMonkey).

This is a **guest workload** — the TS source becomes a wasm Component
loaded inside an actr actor process. It is **not** a client SDK binding;
for the Node.js client API, see `bindings/typescript/`.

## Status: experimental / demo-only

ComponentizeJS embeds the full StarlingMonkey SpiderMonkey build, so the
resulting Component is roughly **10 MB** and incurs a noticeable
cold-start and per-dispatch latency compared with the Rust, TinyGo and C
guests. Use this example for compatibility probes and polyglot demos —
**not** for production actor workloads. The build is intentionally
excluded from CI.

## What it does

- Implements `dispatch(envelope) -> result<list<u8>, actr-error>` —
  echoes the inbound payload prefixed with `"echo: "` (raw bytes, no
  protobuf round-trip).
- Implements `onStart` and uses the host `logMessage` import to emit a
  startup log line.
- The remaining 14 observation hooks are exported as no-ops so the
  Component satisfies the full export surface that the `workload`
  interface requires.

## Toolchain

| Tool                              | Version  | Purpose                            |
|-----------------------------------|----------|------------------------------------|
| `node`                            | >= 20    | npm + TypeScript runtime           |
| `@bytecodealliance/jco`           | 1.18.1   | `componentize` driver              |
| `@bytecodealliance/componentize-js` | 0.20.0 | StarlingMonkey JS engine           |
| `typescript`                      | >= 5.3   | `tsc` transpile to ES module       |
| `wasm-tools`                      | >= 1.219 | Component metadata verification    |

jco is pinned to 1.18.1 to match `bindings/web/package.json` in the
workspace, so all jco-driven pipelines agree on the Component Model
ABI and preview2 adapter versions.

## Build

```bash
./build.sh           # npm install + tsc + jco componentize + verify
./build.sh package   # additionally run `actr build --no-compile` to make .actr
```

The script:

1. `npm install` (idempotent).
2. `npx tsc` — transpiles `src/workload.ts` to `dist/workload.js`
   (ES2022 modules).
3. `npx jco componentize dist/workload.js \
      --wit ../../../core/framework/wit/actr-workload.wit \
      --world-name actr-workload-guest \
      --out echo.wasm`.
4. `wasm-tools component wit echo.wasm` — asserts the emitted
   Component references the `actr:workload` package / world.

## Source layout

- `src/workload.ts` — the workload. Exports one named function per WIT
  export (kebab-case -> camelCase, per ComponentizeJS convention);
  imports `logMessage` from `actr:workload/host@0.1.0`.
- `tsconfig.json` — ES2022 / ESM target matching ComponentizeJS input.
- `package.json` — pins jco 1.18.1 and componentize-js 0.20.0.
- `build.sh` — full pipeline.
- `manifest.toml` — actr packaging metadata (points at `echo.wasm`,
  target `wasm32-wasip2`, no `[build]` section).
- `.gitignore` — ignores `node_modules/`, `dist/`, `*.wasm`, `*.actr`.

## Packaging

`manifest.toml` declares the binary at `echo.wasm` with target
`wasm32-wasip2` (which `actr_pack` resolves to `BinaryKind::Component`).
Because compilation is driven by jco and not Cargo, the manifest
intentionally omits `[build]` — pack with:

```bash
actr build --no-compile -m manifest.toml
```

## Build verification status

Verified locally against:

- node 20.19.x
- `@bytecodealliance/jco@1.18.1`
- `@bytecodealliance/componentize-js@0.20.0`
- `wasm-tools 1.247.0`

Emits a ~12 MB Component whose `wasm-tools component wit` dump shows
`export actr:workload/workload@0.1.0` and imports
`actr:workload/host@0.1.0` plus the StarlingMonkey-required wasi:
`io`, `cli`, `clocks`, `filesystem`, `http` interfaces (the host
provides these via its preview2 adapter at load time).

Not yet run against the actr host at runtime — the `.actr` packaging
path via `actr build --no-compile` is a follow-up. Treat this example
as **source-complete and Component-link-verified**, but runtime
behaviour against the actr host is unverified pending a load test.

## Not to be confused with

- `bindings/typescript/` — Node.js **client** binding (N-API wrapper
  around the actr runtime). Different scope; do not combine.
- `bindings/web/` — browser runtime + jco **transpile** pipeline
  (Component -> ES module for service worker hosting). Uses the same
  jco binary but the opposite direction.

## License

Apache-2.0 — see workspace [LICENSE](../../../LICENSE).
