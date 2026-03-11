#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT_DIR="$ROOT_DIR/client"
SERVER_DIR="$ROOT_DIR/server"

if ! lsof -iTCP:8081 -sTCP:LISTEN -n -P >/dev/null 2>&1; then
  echo "❌ signaling server is not listening on 8081"
  echo "   Start the signaling service from the actrix repository first, for example: cargo run"
  exit 1
fi

echo "📦 Installing test dependencies..."
pnpm install --dir "$ROOT_DIR"
pnpm install --dir "$CLIENT_DIR"
pnpm install --dir "$SERVER_DIR"

echo "🔨 Building WASM bundles..."
"$CLIENT_DIR/build.sh"
"$SERVER_DIR/build.sh"

echo "🚀 Starting dev servers..."
(
  cd "$CLIENT_DIR"
  pnpm dev --host 127.0.0.1 --port 4175
) &
CLIENT_PID=$!
(
  cd "$SERVER_DIR"
  pnpm dev --host 127.0.0.1 --port 4176
) &
SERVER_PID=$!

cleanup() {
  kill "$CLIENT_PID" "$SERVER_PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

sleep 8

echo "🧪 Running browser test..."
cd "$ROOT_DIR"
node test-auto.js
