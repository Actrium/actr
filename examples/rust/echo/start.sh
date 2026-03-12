#!/bin/bash

set -e
set -o pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ACTRIUM_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTRIX_DIR="$ACTRIUM_DIR/actrix"
ECHO_DIR="$WORKSPACE_ROOT/echo"
LOG_DIR="$WORKSPACE_ROOT/logs"
BIN_DIR="$WORKSPACE_ROOT/target/debug"

mkdir -p "$LOG_DIR"

cleanup() {
    echo -e "\n🧹 Cleaning up..."
    [ ! -z "$ACTRIX_PID" ] && kill $ACTRIX_PID 2>/dev/null || true
    [ ! -z "$SERVER_PID" ] && kill $SERVER_PID 2>/dev/null || true
    killall actrix echo-server echo-client 2>/dev/null || true
    wait 2>/dev/null || true
    echo "✅ Cleanup complete"
}
trap cleanup EXIT INT TERM

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Echo Demo —  (Minimal)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Kill existing
killall actrix echo-server echo-client 2>/dev/null || true

echo "📦 Building project..."

cd "$ACTRIX_DIR"
cargo build --bin actrix

cd "$WORKSPACE_ROOT"
cargo build --manifest-path "$ECHO_DIR/server/Cargo.toml"
cargo build --manifest-path "$ECHO_DIR/client/Cargo.toml"

echo -e "\n🔑 Inserting Realm into sqlite database directly..."
REALM_SECRET_PLAIN="rs_Uvj69r18EtEh2FzYlVhnXZ9FOM3ruAQ9"
REALM_SECRET_HASH=$(echo -n "$REALM_SECRET_PLAIN" | sha256sum | cut -d' ' -f1)
mkdir -p "$ACTRIX_DIR/database"
sqlite3 "$ACTRIX_DIR/database/actrix.db" "INSERT OR REPLACE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES (33554432, 'Echo Realm', 'Active', 1, strftime('%s', 'now'), '$REALM_SECRET_HASH');"
echo -e "${GREEN}✅ Realm 33554432 initialized${NC}"

echo -e "\n🚀 Starting Actrix..."
cd "$ACTRIX_DIR"
./target/debug/actrix --config=config.example.toml > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

# Wait for Actrix
echo "⏳ Waiting for Actrix to start..."
MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if lsof -i:8081 > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null; then
        echo -e "${GREEN}✅ Actrix is listening on port 8081${NC}"
        break
    fi
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix not ready after ${MAX_WAIT}s${NC}"
    head -n 50 "$LOG_DIR/actrix.log"
    exit 1
fi

echo -e "\n🚀 Starting Echo Server..."
cd "$ECHO_DIR/server"
"$BIN_DIR/echo-server" --config actr.example.toml > "$LOG_DIR/echo-server.log" 2>&1 &
SERVER_PID=$!

echo "⏳ Waiting for Server to register..."
sleep 3

echo -e "\n🚀 Running Echo Client (Sending Message)..."
cd "$ECHO_DIR/client"
(
  sleep 2
  echo "Hello Actrium!"
  sleep 1
  echo "This is an automated test message."
  sleep 1
  echo "quit"
) | "$BIN_DIR/echo-client" --config actr.example.toml

echo ""