#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "Building runtime-sw..."
(
  cd "$PROJECT_ROOT/crates/runtime-sw"
  ./build.sh
)

echo "Copying runtime-sw artifacts..."
mkdir -p "$SCRIPT_DIR/public"
cp "$PROJECT_ROOT/dist/sw/actr_runtime_sw.js" "$SCRIPT_DIR/public/"
cp "$PROJECT_ROOT/dist/sw/actr_runtime_sw_bg.wasm" "$SCRIPT_DIR/public/"

echo "Runtime-sw artifacts copied to hello-world/public"
