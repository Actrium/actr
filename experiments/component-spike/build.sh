#!/usr/bin/env bash
# Phase 0 spike: build guest Component + host, then run end-to-end.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

echo "=== [1/3] building guest (wasm32-wasip2) ==="
pushd guest >/dev/null
cargo build --release --target wasm32-wasip2
GUEST_WASM="$HERE/guest/target/wasm32-wasip2/release/spike_guest.wasm"
popd >/dev/null

echo
echo "=== [2/3] inspecting guest Component metadata ==="
wasm-tools component wit "$GUEST_WASM" | head -60 || true
SIZE_UNSTRIPPED=$(stat -c %s "$GUEST_WASM")
echo "unstripped size: ${SIZE_UNSTRIPPED} bytes"

# Produce a stripped variant for size comparison.
STRIPPED="$HERE/guest/target/wasm32-wasip2/release/spike_guest.stripped.wasm"
wasm-tools strip "$GUEST_WASM" -o "$STRIPPED" 2>/dev/null || cp "$GUEST_WASM" "$STRIPPED"
SIZE_STRIPPED=$(stat -c %s "$STRIPPED")
echo "stripped   size: ${SIZE_STRIPPED} bytes"

echo
echo "=== [3/3] building + running host ==="
pushd host >/dev/null
cargo run --release --quiet -- "$GUEST_WASM"
popd >/dev/null

echo
echo "=== spike build.sh OK ==="
