#!/bin/bash
set -e

echo "🔨 Building Echo Server WASM..."
echo "   Using server-wasm crate (User Workload + SW Runtime)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_WASM_DIR="$(cd "$SCRIPT_DIR/../server-wasm" && pwd)"

(
  cd "$SERVER_WASM_DIR"
  ./build.sh
)

echo "Echo server WASM artifacts ready in echo/server/public"
