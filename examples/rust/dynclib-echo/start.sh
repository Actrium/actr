#!/bin/bash
# Test script for dynclib-echo example — native cdylib actor loaded via ExecutorAdapter
#
# Demonstrates the full dynclib execution flow:
#   1. Build guest actor as native cdylib (.dylib/.so)
#   2. Host server loads the shared library and registers with signaling
#   3. Client discovers dynclib echo server, sends messages, verifies responses
#
# Usage:
#   ./start.sh              # Use default message "TestMsg"
#   ./start.sh "你好世界"    # Send custom message

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Testing dynclib-echo (Dynclib ExecutorAdapter echo service)"
echo "    Using Actrix as signaling server"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Paths ────────────────────────────────────────────────────────────────

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Actrium root is 3 levels up from WORKSPACE_ROOT (examples/rust):
#   examples/rust → actr/examples → actr → Actrium
ACTRIUM_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTRIX_DIR="$ACTRIUM_DIR/actrix"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
DYNCLIB_ECHO_DIR="$WORKSPACE_ROOT/dynclib-echo"
GUEST_DIR="$DYNCLIB_ECHO_DIR/guest"
SERVER_DIR="$DYNCLIB_ECHO_DIR/server"
CLIENT_DIR="$DYNCLIB_ECHO_DIR/client"

# Ensure ~/.cargo/bin is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required helper scripts
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# Ensure actr.toml files exist
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$SERVER_DIR"
ensure_actr_toml "$CLIENT_DIR"

# Ensure actrix-config.toml exists
ensure_actrix_config "$WORKSPACE_ROOT"

# ── Clean stale database files ───────────────────────────────────────────
# Remove DB files from previous runs so actrix starts with fresh keys.
# Without this, expired signing keys cause "Invalid credential format" errors.
echo ""
echo "🗑️  Cleaning stale database files..."
rm -rf "$WORKSPACE_ROOT/database"
echo -e "${GREEN}✅ Stale database cleaned${NC}"

# ── Cleanup ──────────────────────────────────────────────────────────────

ACTRIX_PID=""
SERVER_PID=""
CLIENT_PID=""

cleanup() {
    echo ""
    echo "🧹 Cleaning up..."

    if [ -n "$ACTRIX_PID" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    if [ -n "$SERVER_PID" ]; then
        echo "Stopping dynclib-echo-server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
    fi

    if [ -n "$CLIENT_PID" ]; then
        echo "Stopping dynclib-echo-client (PID: $CLIENT_PID)"
        kill $CLIENT_PID 2>/dev/null || true
    fi

    wait 2>/dev/null || true
    echo "✅ Cleanup complete"
}

trap cleanup EXIT INT TERM

# ── Step 0: Build native cdylib guest actor ─────────────────────────────

echo ""
echo -e "${BLUE}📦 Building native cdylib guest actor...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$GUEST_DIR"
cargo build --release 2>&1 | tail -5

# Determine cdylib filename based on OS
if [[ "$OSTYPE" == "darwin"* ]]; then
    CDYLIB="libdynclib_echo_guest.dylib"
elif [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "cygwin"* ]]; then
    CDYLIB="dynclib_echo_guest.dll"
else
    CDYLIB="libdynclib_echo_guest.so"
fi

BUILT_LIB="$GUEST_DIR/target/release/$CDYLIB"
if [ ! -f "$BUILT_LIB" ]; then
    echo -e "${RED}❌ Build failed: $BUILT_LIB not found${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Native cdylib built: $(du -h "$BUILT_LIB" | cut -f1) ($CDYLIB)${NC}"

# Return to workspace root
cd "$WORKSPACE_ROOT"

# ── Step 1: Ensure actrix is available ──────────────────────────────────

echo ""
echo "📦 Checking actrix availability..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTRIX_CMD=""
if command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
    echo -e "${GREEN}✅ Actrix found: $(which actrix)${NC}"
elif [ -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
    echo -e "${GREEN}✅ Actrix found: $ACTRIX_CMD${NC}"
elif [ -f "$ACTRIX_DIR/target/release/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
    echo -e "${GREEN}✅ Actrix found: $ACTRIX_CMD${NC}"
else
    echo -e "${YELLOW}⚠️  Actrix not found in PATH or build directory. Attempting build...${NC}"
    if [ -d "$ACTRIX_DIR" ]; then
        cd "$ACTRIX_DIR"
        cargo build 2>&1 | tail -5
        if [ -f "$ACTRIX_DIR/target/debug/actrix" ]; then
            ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
            mkdir -p ~/.cargo/bin
            cp "$ACTRIX_CMD" ~/.cargo/bin/actrix
            chmod +x ~/.cargo/bin/actrix
            ACTRIX_CMD="actrix"
        fi
        cd "$WORKSPACE_ROOT"
    fi

    if [ -z "$ACTRIX_CMD" ]; then
        echo -e "${RED}❌ Actrix not available. Install it first or build from $ACTRIX_DIR${NC}"
        exit 1
    fi
fi

# ── Step 2: Start actrix ────────────────────────────────────────────────

echo ""
echo "🚀 Starting actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

$ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "Actrix started (PID: $ACTRIX_PID)"
echo "Waiting for actrix to be ready..."

MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi

    if lsof -i:8081 > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null; then
        echo -e "${GREEN}✅ Actrix is running and listening on port 8081${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix not listening on port 8081 after ${MAX_WAIT} seconds${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# ── Step 2.5: Setup realms ──────────────────────────────────────────────

echo ""
echo "🔑 Setting up realms in actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

sleep 2

# Extract realm IDs from actr.toml files
SERVER_REALM=$(grep -E 'realm_id\s*=' "$SERVER_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
CLIENT_REALM=$(grep -E 'realm_id\s*=' "$CLIENT_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')

# Insert realms directly into SQLite (same approach as actrix fullstack tests)
ACTRIX_DB="$WORKSPACE_ROOT/database/actrix.db"

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix database not found at $ACTRIX_DB${NC}"
    echo "Actrix may not have started properly."
    exit 1
fi

for REALM_ID in $SERVER_REALM $CLIENT_REALM; do
    echo "  Creating realm $REALM_ID..."
    sqlite3 "$ACTRIX_DB" \
        "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'dynclib-echo-realm', 'Active', 1, strftime('%s','now'), '');"
done

echo -e "${GREEN}✅ Realms setup completed (realm IDs: $SERVER_REALM, $CLIENT_REALM)${NC}"

# ── Step 3: Build host binaries ─────────────────────────────────────────

echo ""
echo "🔨 Building host binaries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin dynclib-echo-server --bin dynclib-echo-client 2>&1; then
    echo -e "${RED}❌ Failed to build binaries${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binaries built successfully${NC}"

# ── Step 4: Start dynclib echo server ───────────────────────────────────

echo ""
echo "🚀 Starting dynclib-echo-server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-info}" cargo run --bin dynclib-echo-server > "$LOG_DIR/dynclib-echo-server.log" 2>&1 &
SERVER_PID=$!

echo "Server started (PID: $SERVER_PID)"
echo "Waiting for dynclib server to register..."

MAX_WAIT=15
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}❌ Dynclib server failed to start${NC}"
        cat "$LOG_DIR/dynclib-echo-server.log"
        exit 1
    fi

    if grep -q "Dynclib Echo Server fully started\|ActrNode started" "$LOG_DIR/dynclib-echo-server.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Dynclib server is running and registered${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Server may not have fully registered, but continuing...${NC}"
fi

sleep 2

# ── Step 5: Run client with test input ──────────────────────────────────

echo ""
echo "🚀 Running dynclib-echo-client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ -n "$1" ]; then
    TEST_INPUT="$1"
else
    TEST_INPUT="TestMsg"
fi

echo "Sending test message: \"$TEST_INPUT\""

(
    sleep 3
    echo "$TEST_INPUT"
    sleep 2
    echo "quit"
) | RUST_LOG="${RUST_LOG:-info}" cargo run --bin dynclib-echo-client > "$LOG_DIR/dynclib-echo-client.log" 2>&1 &
CLIENT_PID=$!

# Wait for client to finish (max 15 seconds)
COUNTER=0
while kill -0 $CLIENT_PID 2>/dev/null && [ $COUNTER -lt 15 ]; do
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if kill -0 $CLIENT_PID 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Client still running after 15 seconds, killing...${NC}"
    kill $CLIENT_PID 2>/dev/null || true
fi

# ── Step 6: Verify output ───────────────────────────────────────────────

echo ""
echo "🔍 Verifying output..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if grep -q "\[Received reply\].*Echo: $TEST_INPUT" "$LOG_DIR/dynclib-echo-client.log"; then
    echo -e "${GREEN}✅ Test PASSED: Dynclib echo server response received${NC}"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🎉 Dynclib Echo test completed successfully!"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "✅ Validated:"
    echo "   • Native cdylib guest actor compiled for host platform"
    echo "   • DynclibHost load → instantiate → init"
    echo "   • ActrNode with Dynclib ExecutorAdapter"
    echo "   • Real distributed Actor communication (client ↔ dynclib server)"
    echo ""
    echo "Client output:"
    cat "$LOG_DIR/dynclib-echo-client.log" | grep "Received reply" || true
    echo ""
    echo "📖 View full logs:"
    echo "   cat $LOG_DIR/dynclib-echo-client.log  # Client logs"
    echo "   cat $LOG_DIR/dynclib-echo-server.log  # Dynclib server logs"
    echo "   tail -f $LOG_DIR/actrix.log           # Actrix logs"
    echo ""
    exit 0
else
    echo -e "${RED}❌ Test FAILED: Expected dynclib echo server response not found${NC}"
    echo -e "${RED}   Looking for: [Received reply] Echo: $TEST_INPUT${NC}"
    echo ""
    echo "Client output:"
    cat "$LOG_DIR/dynclib-echo-client.log"
    echo ""
    echo "Server output (last 30 lines):"
    tail -30 "$LOG_DIR/dynclib-echo-server.log"
    echo ""
    echo "Actrix output (last 30 lines):"
    tail -30 "$LOG_DIR/actrix.log"
    exit 1
fi
