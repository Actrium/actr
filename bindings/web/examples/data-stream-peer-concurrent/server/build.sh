#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WASM_DIR="$SCRIPT_DIR/wasm"

echo "Building data-stream server wasm..."
(
  cd "$WASM_DIR"
  ./build.sh
)
