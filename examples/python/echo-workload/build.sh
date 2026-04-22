#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Build the echo Python workload as a Component Model wasm via
# componentize-py, and optionally pack it into a signed `.actr`.
#
# Required tools (versions known to work against this WIT contract):
#
#   - python3           >= 3.10    (componentize-py host interpreter;
#                                    distinct from the CPython *guest*
#                                    interpreter that componentize-py
#                                    bundles into the Component)
#   - pip               >= 23
#   - componentize-py   == 0.17.2  (alpha; see requirements.txt for notes)
#   - wasm-tools        >= 1.219   (component metadata verification)
#
# Optional:
#
#   - actr CLI                     (workspace root: cargo run -p actr -- build ...)
#
# Toolchain reality: componentize-py downloads a prebuilt CPython WASM
# interpreter on first use (cached under ~/.cache/componentize-py or the
# pip wheels directory). Internet access is required the first time.
#
# Size warning: the resulting Component is roughly 10 MB because it
# embeds the full CPython 3.12 interpreter plus the standard library
# subset that componentize-py's bundler selects. For size-sensitive
# deployments, prefer the Go / C / Rust examples.
#
# Usage:
#   ./build.sh              # install deps + bindings + componentize + verify
#   ./build.sh package      # additionally run `actr build --no-compile`

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WIT_FILE="${HERE}/../../../core/framework/wit/actr-workload.wit"
WORLD="actr-workload-guest"

BINDINGS_DIR="${HERE}/bindings"
DIST_DIR="${HERE}/dist"
OUT_WASM="${DIST_DIR}/echo-python-0.1.0-wasm32-wasip2.wasm"

VENV_DIR="${HERE}/.venv"

if [[ ! -f "${WIT_FILE}" ]]; then
    echo "error: WIT contract not found at ${WIT_FILE}" >&2
    exit 1
fi

# ── 0. Prepare an isolated Python venv ───────────────────────────────────────
#
# componentize-py and its bundled CPython-WASM wheels are heavy; keep
# them out of the user's global site-packages.
if [[ ! -d "${VENV_DIR}" ]]; then
    echo "[0/4] creating venv at ${VENV_DIR} ..."
    python3 -m venv "${VENV_DIR}"
fi
# shellcheck disable=SC1091
source "${VENV_DIR}/bin/activate"

echo "[0/4] installing componentize-py (alpha) ..."
pip install --upgrade pip >/dev/null
pip install -r "${HERE}/requirements.txt"

# ── 1. Generate Python bindings from WIT ─────────────────────────────────────
#
# The `bindings` subcommand emits a Python package tree mirroring the WIT
# package layout. For `package actr:workload@0.1.0` with world
# `actr-workload-guest`, the output under bindings/ is:
#
#   actr_workload/__init__.py
#   actr_workload/exports/workload.py   <- Workload protocol class
#   actr_workload/imports/host.py       <- host-side imports
#   actr_workload/types.py              <- record / variant types
#
# The exact path mapping is controlled by componentize-py and may shift
# between 0.16 / 0.17 / 0.18. The Workload subclass in workload.py tracks
# 0.17.x — if the imports break on upgrade, inspect the generated tree
# and adjust `workload.py`'s `from actr_workload...` lines.

echo "[1/4] componentize-py bindings ..."
rm -rf "${BINDINGS_DIR}"
componentize-py \
    -w "${WORLD}" \
    -d "${WIT_FILE}" \
    bindings "${BINDINGS_DIR}"

# ── 2. Bundle workload.py + bindings + CPython into a Component ──────────────
#
# The `componentize` subcommand takes the module name (here `workload`,
# which resolves to workload.py in the working directory) and produces
# a wasm32-wasip2 Component that exports the world's workload interface.
# componentize-py embeds a CPython 3.12 WASM interpreter plus the subset
# of the stdlib that its bundler decides is reachable — hence ~10 MB.

echo "[2/4] componentize-py componentize ..."
mkdir -p "${DIST_DIR}"
(
    cd "${HERE}"
    componentize-py \
        -w "${WORLD}" \
        -d "${WIT_FILE}" \
        componentize workload \
        -o "${OUT_WASM}"
)

# ── 3. Verify world / interfaces via wasm-tools ──────────────────────────────
echo "[3/4] wasm-tools component wit (verify world) ..."
wasm-tools component wit "${OUT_WASM}" | tee "${DIST_DIR}/echo-python.wit.txt"

if grep -q "actr:workload" "${DIST_DIR}/echo-python.wit.txt"; then
    echo
    echo "OK: emitted Component references actr:workload interfaces"
else
    echo "FAIL: actr:workload interfaces not found in component metadata" >&2
    exit 1
fi

# ── 4. Report size (the 10 MB warning) ───────────────────────────────────────
echo "[4/4] size report ..."
ls -lh "${OUT_WASM}" | awk '{ print "    component size: " $5 "  (" $NF ")" }'

# ── Optional: pack into .actr ────────────────────────────────────────────────
if [[ "${1:-}" == "package" ]]; then
    echo
    echo "[+] actr build --no-compile ..."
    ACTR_ROOT="${HERE}/../../.."
    (
        cd "${HERE}"
        cargo run --manifest-path "${ACTR_ROOT}/Cargo.toml" -p actr -- \
            build --no-compile -m manifest.toml
    )
fi

echo
echo "Done. Component at: ${OUT_WASM}"
