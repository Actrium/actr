#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT_WASM_DIR="$(cd "$SCRIPT_DIR/../client-wasm" && pwd)"

echo "Building echo-client-wasm (Local Handler + SW Runtime)..."
(
  cd "$CLIENT_WASM_DIR"
  ./build.sh
)

echo "Echo client WASM artifacts ready in echo/client/public"
