#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Build the echo Go workload as a Component Model wasm and pack it into
# a signed `.actr` package.
#
# Required tools (versions known to work against this WIT contract):
#
#   - tinygo            >= 0.34.0   (wasip2 target, Component Model linker)
#   - wit-bindgen-go    >= 0.6.0    (Bytecode Alliance Go bindings generator)
#   - wasm-tools        >= 1.219    (component metadata round-trip + verify)
#   - go                >= 1.23     (TinyGo dispatches to the system go for deps)
#
# Optional (for `actr build --no-compile` packaging):
#
#   - actr CLI          (workspace root: cargo run -p actr-cli -- build ...)
#
# Usage:
#   ./build.sh           # generate + compile + verify world
#   ./build.sh package   # also run `actr build --no-compile` to produce .actr

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WIT_FILE="${HERE}/../../../core/framework/wit/actr-workload.wit"
OUT_WASM="${HERE}/dist/echo-go-0.1.0-wasm32-wasip2.wasm"

if [[ ! -f "${WIT_FILE}" ]]; then
    echo "error: WIT contract not found at ${WIT_FILE}" >&2
    exit 1
fi

# ── 1. Generate Go bindings from WIT ─────────────────────────────────────────
#
# wit-bindgen-go drops a tree under `gen/` matching the WIT package shape:
#
#   gen/
#   ├── actr/workload/types/         <- record / variant types
#   ├── actr/workload/host/          <- imported host functions
#   ├── actr/workload/workload/      <- exported workload interface
#   └── actr-workload-guest/         <- world entry-point glue
#
# The generator is idempotent — committing `gen/` would also be valid, but
# we keep it out of the tree (.gitignore) since it is reproducible.

echo "[1/4] wit-bindgen-go generate ..."
rm -rf "${HERE}/gen"
wit-bindgen-go generate \
    --world actr-workload-guest \
    --out "${HERE}/gen" \
    "${WIT_FILE}"

# ── 2. Resolve Go module dependencies ─────────────────────────────────────────
echo "[2/4] go mod tidy ..."
( cd "${HERE}" && go mod tidy )

# ── 3. Compile to wasm32-wasip2 Component ─────────────────────────────────────
#
# `tinygo build -target=wasip2` emits a Component directly (TinyGo embeds the
# wasi:cli + wasi:io WIT and runs wasm-component-ld internally). The output is
# already a Component Model binary — no separate wasm-tools `component new`
# step is needed.

echo "[3/4] tinygo build (wasip2) ..."
mkdir -p "${HERE}/dist"
( cd "${HERE}" && tinygo build \
    -target=wasip2 \
    -wit-package "${WIT_FILE}" \
    -wit-world actr-workload-guest \
    -o "${OUT_WASM}" \
    ./... )

# ── 4. Verify world / interfaces ─────────────────────────────────────────────
echo "[4/4] wasm-tools component wit (verify world) ..."
wasm-tools component wit "${OUT_WASM}" | tee "${HERE}/dist/echo-go.wit.txt"

if grep -q "actr-workload-guest" "${HERE}/dist/echo-go.wit.txt"; then
    echo
    echo "OK: emitted Component implements world actr-workload-guest"
else
    echo "FAIL: world actr-workload-guest not found in component metadata" >&2
    exit 1
fi

# ── Optional: pack into .actr ────────────────────────────────────────────────
if [[ "${1:-}" == "package" ]]; then
    echo
    echo "[+] actr build --no-compile ..."
    ACTR_ROOT="${HERE}/../../.."
    ( cd "${HERE}" && cargo run --manifest-path "${ACTR_ROOT}/Cargo.toml" -p actr -- \
        build --no-compile -m manifest.toml )
fi

echo
echo "Done. Component at: ${OUT_WASM}"
