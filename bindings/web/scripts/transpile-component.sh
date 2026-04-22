#!/bin/bash
# transpile-component.sh — run `jco transpile` on a Component Model .wasm
# artifact, producing the ES module + core .wasm + JS glue bundle that the
# Service Worker runtime loads at run time.
#
# Usage:
#   scripts/transpile-component.sh <component.wasm> <out-dir> [--name <basename>]
#
# Arguments:
#   <component.wasm>  Path to the Component Model wasm binary.
#   <out-dir>         Directory that will hold the transpile output. The
#                     Service Worker default convention is
#                     `<package_url>.jco/` (sibling of the `.actr` package).
#   --name <basename> (optional) Rename the transpile output to
#                     `<basename>.js` / `<basename>.core.wasm` instead of
#                     the wasm file's stem. The browser SW defaults to
#                     loading `<package>.jco/guest.js`, so the recommended
#                     invocation is `--name guest`.
#
# Notes:
#   - Input must be a Component Model binary (built via `cargo build --target
#     wasm32-wasip2` + `wasm-component-ld`). Core-wasm modules are rejected by
#     `jco transpile`.
#   - `--instantiation async` is required because the actr WIT contract is
#     driven by `wit-bindgen ... async: true` and emits `context.get` async-
#     ABI primitives in the guest core wasm (see experiments/component-spike-
#     async/REPORT.md).
#   - Generated files after renaming:
#       <out-dir>/<name>.js          — ES module with `instantiate()` entry
#       <out-dir>/<name>.d.ts        — TypeScript types
#       <out-dir>/<name>.core.wasm   — user's guest core module
#       <out-dir>/<name>.coreN.wasm  — jco adapter modules
#       <out-dir>/interfaces/*.d.ts  — per-interface types

set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "Usage: $0 <component.wasm> <out-dir> [--name <basename>]" >&2
    exit 1
fi

INPUT="$1"
OUT_DIR="$2"
shift 2

RENAME=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --name)
            RENAME="$2"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

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

# Derive the stem jco used (matches the input .wasm's basename without
# extension) and optionally rename every emitted artifact to the caller-
# supplied name. The rename also rewrites the `compileCore('<stem>.core.wasm')`
# calls inside the .js module so the renamed core wasm files continue to
# resolve correctly.
STEM="$(basename "$INPUT" .wasm)"

if [[ -n "$RENAME" && "$RENAME" != "$STEM" ]]; then
    echo "[transpile-component] renaming outputs: ${STEM}.* -> ${RENAME}.*"
    for f in "$OUT_DIR/${STEM}".*; do
        [[ -e "$f" ]] || continue
        suffix="${f#"$OUT_DIR/${STEM}"}"
        mv "$f" "$OUT_DIR/${RENAME}${suffix}"
    done

    # Rewrite core-wasm filename references inside the .js module. jco
    # generates `compileCore('name.core.wasm')` and `compileCore('name.coreN.wasm')`
    # calls; after rename, both the core.wasm and the adapter coreN.wasm
    # files carry the new stem, so every `${STEM}` prefix in those string
    # literals needs updating.
    JS_FILE="$OUT_DIR/${RENAME}.js"
    if [[ -f "$JS_FILE" ]]; then
        # macOS sed needs a backup-extension arg; GNU sed accepts `-i` empty.
        # Use a portable wrapper: python for the in-place rewrite.
        python3 - <<PYEOF
import pathlib, re
p = pathlib.Path("$JS_FILE")
src = p.read_text()
# Match ' STEM.(core|coreN).wasm' inside single-quoted strings.
src = re.sub(
    r"'${STEM}(\.(?:core\d*)\.wasm)'",
    r"'${RENAME}\1'",
    src,
)
# Also cover double-quoted strings just in case.
src = re.sub(
    r'"${STEM}(\.(?:core\d*)\.wasm)"',
    r'"${RENAME}\1"',
    src,
)
p.write_text(src)
PYEOF
    fi
fi

echo "[transpile-component] done"
ls -lh "$OUT_DIR"
