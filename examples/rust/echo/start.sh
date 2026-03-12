#!/bin/bash


set -e
set -o pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
ECHO_DIR="$WORKSPACE_ROOT/echo"
LOG_DIR="$WORKSPACE_ROOT/logs"

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
cd "$ECHO_DIR/server"
cargo build
cd "$ECHO_DIR/client"
cargo build
cd "$ACTRIX_DIR"
cargo build --bin actrix

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

echo -e "\n🔑 Inserting Realm into sqlite database directly..."
sqlite3 "$ACTRIX_DIR/database/actrix.db" "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES (33554432, 'Echo Realm', 'Active', 1, strftime('%s', 'now'), '');"
echo -e "${GREEN}✅ Realm 33554432 initialized${NC}"

echo -e "\n🚀 Starting Echo Server..."
cd "$ECHO_DIR/server"
../../target/debug/echo-server --config=actr.toml > "$LOG_DIR/echo-server.log" 2>&1 &
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
) | ../../target/debug/echo-client --config actr.toml

echo ""
