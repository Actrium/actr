#!/bin/bash
# Test script for shell-actr-echo example - Using actrix as signaling server
# Tests the full Shell → Workload RPC flow via ActrRef
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
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Testing shell-actr-echo (Shell ↔ Workload via ActrRef)"
echo "    Using Actrix as signaling server"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine paths based on script location
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ACTR_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTOR_RTC_DIR="$(cd "$ACTR_ROOT/.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"

# Cleanup function
cleanup() {
    echo ""
    echo "🧹 Cleaning up..."

    # Kill actrix
    if [ ! -z "$ACTRIX_PID" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    # Kill echo server
    if [ ! -z "$SERVER_PID" ]; then
        echo "Stopping shell-actr-echo/server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
    fi

    # Kill client app
    if [ ! -z "$CLIENT_PID" ]; then
        echo "Stopping shell-actr-echo/client (PID: $CLIENT_PID)"
        kill $CLIENT_PID 2>/dev/null || true
    fi

    wait 2>/dev/null || true

    echo "✅ Cleanup complete"
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Step 1: Build all components
echo ""
echo "📦 Building components..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check/Build actrix
ACTRIX_CMD=""

if command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
    echo "✅ Using installed actrix command: $(which actrix)"
elif [ -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
    echo "✅ Using local actrix build: $ACTRIX_CMD"
elif [ -f "$ACTRIX_DIR/target/release/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
    echo "✅ Using local actrix build: $ACTRIX_CMD"
elif [ -d "$ACTRIX_DIR" ]; then
    echo -e "${YELLOW}⚠️  actrix not found, building from source...${NC}"
    cd "$ACTRIX_DIR"
    cargo build 2>&1 | tail -5
    ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
    echo "✅ Built actrix: $ACTRIX_CMD"
    cd "$SCRIPT_DIR"
else
    echo -e "${RED}❌ Cannot find actrix directory at $ACTRIX_DIR${NC}"
    echo "Please ensure actrix project exists at: $ACTRIX_DIR"
    exit 1
fi

echo "Building shell-actr-echo/server..."
cd "$SCRIPT_DIR/server"
cargo build 2>&1 | tail -3

echo "Building shell-actr-echo/client..."
cd "$SCRIPT_DIR/client"
cargo build 2>&1 | tail -3

cd "$SCRIPT_DIR"

echo -e "${GREEN}✅ All components built${NC}"

# Step 2: Start actrix
echo ""
echo "🚀 Starting actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

$ACTRIX_CMD --config "$SCRIPT_DIR/actrix-config.toml" > /tmp/actrix.log 2>&1 &
ACTRIX_PID=$!

echo "Actrix started (PID: $ACTRIX_PID)"
echo "Waiting for actrix to be ready..."
sleep 3

if ! kill -0 $ACTRIX_PID 2>/dev/null; then
    echo -e "${RED}❌ Actrix failed to start${NC}"
    cat /tmp/actrix.log
    exit 1
fi

echo -e "${GREEN}✅ Actrix is running${NC}"

# Step 3: Start shell-actr-echo/server
echo ""
echo "🚀 Starting shell-actr-echo/server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-info}" "$SCRIPT_DIR/server/target/debug/echo-real-server" > /tmp/shell-actr-echo-server.log 2>&1 &
SERVER_PID=$!

echo "Server started (PID: $SERVER_PID)"
echo "Waiting for server to register..."
sleep 3

if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo -e "${RED}❌ Server failed to start${NC}"
    cat /tmp/shell-actr-echo-server.log
    exit 1
fi

echo -e "${GREEN}✅ Server is running${NC}"

# Step 4: Run shell-actr-echo/client with test input
echo ""
echo "🚀 Running shell-actr-echo/client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Get test message from user or use argument/default
if [ -n "$1" ]; then
    # Use command line argument
    TEST_INPUT="$1"
else
    # Use default message
    TEST_INPUT="TestMsg"
fi

echo "Sending test message: \"$TEST_INPUT\""

# Run client app with test input
(
    sleep 2
    echo "$TEST_INPUT"
    sleep 2
    echo "quit"
) | RUST_LOG="${RUST_LOG:-info}" "$SCRIPT_DIR/client/target/debug/echo-real-client-app" > /tmp/shell-actr-echo-client.log 2>&1 &
CLIENT_PID=$!

# Wait for client to finish (max 10 seconds)
COUNTER=0
while kill -0 $CLIENT_PID 2>/dev/null && [ $COUNTER -lt 10 ]; do
    sleep 1
    COUNTER=$((COUNTER + 1))
done

# Check if client is still running (should have exited)
if kill -0 $CLIENT_PID 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Client still running after 10 seconds, killing...${NC}"
    kill $CLIENT_PID 2>/dev/null || true
fi

# Step 5: Verify output
echo ""
echo "🔍 Verifying output..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if the response contains the server's echo reply (NOT just user input)
# Server responds with "Echo: <message>", client prints "[Received reply] Echo: <message>"
if grep -q "\[Received reply\].*Echo: $TEST_INPUT" /tmp/shell-actr-echo-client.log; then
    echo -e "${GREEN}✅ Test PASSED: Server echo response received${NC}"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🎉 Test completed successfully!"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "✅ Validated:"
    echo "   • Shell → Workload RPC via ActrRef"
    echo "   • Real distributed Actor communication"
    echo "   • Using actrix as signaling server"
    echo ""
    echo "Client app output:"
    cat /tmp/shell-actr-echo-client.log | grep "Received reply" || true
    echo ""
    echo "📖 View full logs:"
    echo "   cat /tmp/shell-actr-echo-client.log  # Client logs"
    echo "   cat /tmp/shell-actr-echo-server.log  # Server logs"
    echo "   tail -f /tmp/actrix.log              # Actrix logs"
    echo ""
    exit 0
else
    echo -e "${RED}❌ Test FAILED: Expected server echo response not found${NC}"
    echo -e "${RED}   Looking for: [Received reply] Echo: $TEST_INPUT${NC}"
    echo ""
    echo "Client app output:"
    cat /tmp/shell-actr-echo-client.log
    echo ""
    echo "Server output:"
    cat /tmp/shell-actr-echo-server.log | tail -20
    echo ""
    echo "Actrix output:"
    tail -30 /tmp/actrix.log
    exit 1
fi
