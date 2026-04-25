#!/bin/bash
set -euo pipefail

# Clear host-toolchain RUSTFLAGS so host-linker flags (e.g. `-fuse-ld=mold`
# from a global ~/.cargo/config.toml `[build] rustflags`) don't leak into
# the wasm32 target build — rust-lld errors out on them.
export RUSTFLAGS=""
export CARGO_ENCODED_RUSTFLAGS=""

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
