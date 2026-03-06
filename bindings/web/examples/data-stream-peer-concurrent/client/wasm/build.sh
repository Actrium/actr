#!/bin/bash
set -euo pipefail

TMPDIR="${TMPDIR:-$(pwd)/.tmp}"
export TMPDIR
mkdir -p "$TMPDIR"

OUT_DIR="../public"
rm -f "$OUT_DIR"/data_stream_client*.wasm "$OUT_DIR"/data_stream_client*.js "$OUT_DIR"/data_stream_client*.d.ts

wasm-pack build \
  --target no-modules \
  --out-dir "$OUT_DIR" \
  --out-name data_stream_client \
  --release

rm -f "$OUT_DIR/package.json" "$OUT_DIR/.gitignore"

echo "Built data_stream_client wasm to $OUT_DIR"
