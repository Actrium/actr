# echo-workload (TypeScript)

Minimal ACTR workload authored in TypeScript, componentized as a
`wasm32-wasip2` Component Model module, and packaged as an `.actr` artifact.

The example uses the local package-first workload authoring package at
`../../../bindings/typescript/actr-workload`. The workload WIT contract is
resolved explicitly from `../../../core/framework/wit/actr-workload.wit`.

## Install

```bash
npm install
```

## Build

```bash
npm run build
```

This compiles `src/workload.ts` to `dist/workload.js`.

## Componentize

```bash
npm run componentize
```

This runs `actr-workload-ts componentize` and writes:

```text
dist/echo-typescript-0.1.0-wasm32-wasip2.wasm
```

## Package

```bash
npm run package
```

The package step runs `actr build --manifest manifest.toml --no-compile`.
Compilation is handled by the TypeScript workload componentizer, so the ACTR
packaging step only wraps the generated WASI Preview 2 component.
