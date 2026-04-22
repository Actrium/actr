#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Build the TypeScript echo workload as a Component Model wasm.
#
# Toolchain (pinned to match the workspace `bindings/web/package.json`):
#
#   - node                                 >= 20
#   - @bytecodealliance/jco                1.18.1   (componentize driver)
#   - @bytecodealliance/componentize-js    0.20.0   (StarlingMonkey engine)
#   - wasm-tools                           >= 1.219 (Component metadata verify)
#
# ComponentizeJS uses the StarlingMonkey SpiderMonkey build internally, which
# bloats the resulting Component to roughly 10 MB. This example is a demo /
# compatibility probe — not a production guest target.
#
# Usage:
#   ./build.sh           # compile TS + componentize + verify world
#   ./build.sh package   # additionally run `actr build --no-compile`

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WIT_FILE="${HERE}/../../../core/framework/wit/actr-workload.wit"
JS_SOURCE="${HERE}/dist/workload.js"
OUT_WASM="${HERE}/echo.wasm"

if [[ ! -f "${WIT_FILE}" ]]; then
    echo "error: WIT contract not found at ${WIT_FILE}" >&2
    exit 1
fi

# ── 1. npm install (idempotent) ──────────────────────────────────────────────
if [[ ! -d "${HERE}/node_modules" ]]; then
    echo "[1/4] npm install ..."
    ( cd "${HERE}" && npm install --no-audit --no-fund )
else
    echo "[1/4] npm install ... (cached)"
fi

# ── 2. TypeScript -> ES module ───────────────────────────────────────────────
#
# jco componentize consumes a plain ES module; we transpile TS first so that
# `src/workload.ts` stays the authoring entry point.

echo "[2/4] tsc ..."
( cd "${HERE}" && npx tsc )

if [[ ! -f "${JS_SOURCE}" ]]; then
    echo "error: tsc produced no ${JS_SOURCE}" >&2
    exit 1
fi

# ── 3. jco componentize -> Component Model wasm ─────────────────────────────
#
# Flags:
#   --wit          path to the actr workload contract
#   --world-name   world to build (matches the WIT `world actr-workload-guest`)
#   --out          Component output path
#
# StarlingMonkey always pulls wasi:cli/stdio/clocks/filesystem/io/http
# imports regardless of `--disable <feature>` (which only trims
# engine-level shims); the actr host provides those via the preview2
# adapter at load time.

echo "[3/4] jco componentize ..."
( cd "${HERE}" && npx jco componentize "${JS_SOURCE}" \
    --wit "${WIT_FILE}" \
    --world-name actr-workload-guest \
    --out "${OUT_WASM}" )

# ── 4. Verify emitted world ──────────────────────────────────────────────────
echo "[4/4] wasm-tools component wit (verify world) ..."
WIT_DUMP="${HERE}/dist/echo-ts.wit.txt"
wasm-tools component wit "${OUT_WASM}" | tee "${WIT_DUMP}" >/dev/null

if grep -q "actr-workload-guest\|actr:workload" "${WIT_DUMP}"; then
    echo
    echo "OK: emitted Component references actr:workload world"
    echo "    size: $(du -h "${OUT_WASM}" | cut -f1)"
else
    echo "FAIL: actr:workload world not found in Component metadata" >&2
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
