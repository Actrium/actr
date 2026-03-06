#!/bin/bash
set -e

echo "🔨 Building Echo Server WASM..."
echo "   Using wasm crate (User Workload + SW Runtime)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WASM_DIR="$SCRIPT_DIR/wasm"

(
  cd "$WASM_DIR"
  ./build.sh
)

echo "Echo server WASM artifacts ready in echo/server/public"
