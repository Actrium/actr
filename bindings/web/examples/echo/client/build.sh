#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WASM_DIR="$SCRIPT_DIR/wasm"

echo "Building client wasm (Local Handler + SW Runtime)..."
(
  cd "$WASM_DIR"
  ./build.sh
)

echo "Echo client WASM artifacts ready in echo/client/public"
