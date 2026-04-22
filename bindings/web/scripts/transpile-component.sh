#!/bin/bash
# transpile-component.sh — run `jco transpile` on a Component Model .wasm
# artifact, producing the ES module + core .wasm + JS glue bundle that the
# Service Worker runtime loads at run time.
#
# Usage:
#   scripts/transpile-component.sh <component.wasm> <out-dir>
#
# Notes:
#   - Input must be a Component Model binary (built via `cargo build --target
#     wasm32-wasip2` + `wasm-component-ld`). Core-wasm modules are rejected by
#     `jco transpile`.
#   - `--instantiation async` is required because the actr WIT contract is
#     driven by `wit-bindgen ... async: true` and emits `context.get` async-
#     ABI primitives in the guest core wasm (see experiments/component-spike-
#     async/REPORT.md).
#   - Generated files:
#       <out-dir>/<name>.js          — ES module with `instantiate()` entry
#       <out-dir>/<name>.d.ts        — TypeScript types
#       <out-dir>/<name>.core.wasm   — user's guest core module
#       <out-dir>/<name>.coreN.wasm  — jco adapter modules
#       <out-dir>/interfaces/*.d.ts  — per-interface types

set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "Usage: $0 <component.wasm> <out-dir>" >&2
    exit 1
fi

INPUT="$1"
OUT_DIR="$2"

if [[ ! -f "$INPUT" ]]; then
    echo "Error: input component not found: $INPUT" >&2
    exit 1
fi

mkdir -p "$OUT_DIR"

# Pin the jco version to match package.json devDependencies so local runs and
# CI agree. The devDependency ensures jco resolves offline in a `npm ci`
# workspace; the explicit `npx` invocation keeps this script usable from a
# clone that hasn't yet installed the workspace devDependencies.
JCO_VERSION="1.18.1"

echo "[transpile-component] running jco@${JCO_VERSION} transpile on ${INPUT} -> ${OUT_DIR}"

npx --yes "@bytecodealliance/jco@${JCO_VERSION}" transpile \
    "$INPUT" \
    --instantiation async \
    --out-dir "$OUT_DIR"

echo "[transpile-component] done"
ls -lh "$OUT_DIR"
