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

# Determine paths and switch to workspace root
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Optional paths for local source builds (only used if binaries are not in PATH)
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd || echo "")"
ACTRIX_DIR="${ACTRIX_DIR:-${PROJECT_ROOT:+$PROJECT_ROOT/actrix}}"
ACTR_CLI_DIR="${ACTR_CLI_DIR:-${PROJECT_ROOT:+$PROJECT_ROOT/actr}}"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
ECHO_SERVER_DIR="$WORKSPACE_ROOT/shell-actr-echo/server"
ECHO_CLIENT_DIR="$WORKSPACE_ROOT/shell-actr-echo/client"
PROTO_DIR="$WORKSPACE_ROOT/shell-actr-echo/proto"

# Switch to workspace root and stay there
cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required CLI tools
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"

# Ensure actr.toml files exist
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# Ensure actr.toml files exist for server and client
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$ECHO_SERVER_DIR"
ensure_actr_toml "$ECHO_CLIENT_DIR"

# Ensure actrix-config.toml exists
ensure_actrix_config "$WORKSPACE_ROOT"

echo ""
echo "🧰 Checking required CLI tools..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_cargo_bin "protoc-gen-prost" "protoc-gen-prost" "$LOG_DIR"
ensure_cargo_bin "protoc-gen-actrframework" "actr-framework-protoc-codegen" "$LOG_DIR"
ensure_cargo_bin "actr" "actr-cli" "$LOG_DIR"

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

# Step 0: Generate server code (protobuf + actor glue)
echo ""
echo "🛠️ Generating server code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_GEN_CMD=""

if command -v actr > /dev/null 2>&1; then
    ACTR_GEN_CMD="actr"
elif [ -n "$ACTR_CLI_DIR" ] && [ -x "$ACTR_CLI_DIR/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTR_CLI_DIR/target/debug/actr"
elif [ -n "$ACTR_CLI_DIR" ] && [ -x "$ACTR_CLI_DIR/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTR_CLI_DIR/target/release/actr"
else
    echo -e "${RED}❌ actr generator not found (expected 'actr' in PATH or built locally)${NC}"
    echo "Please install actr-cli:"
    echo "  cargo install actr-cli"
    exit 1
fi

if [ ! -d "$PROTO_DIR" ]; then
    echo -e "${RED}❌ Proto directory not found at $PROTO_DIR${NC}"
    exit 1
fi

cd "$ECHO_SERVER_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-echo-server.log"
$ACTR_GEN_CMD install > /dev/null 2>&1 || true
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"

echo -e "${GREEN}✅ actr gen completed (server code refreshed)${NC}"

# Step 0b: Generate client code (protobuf + actor glue)
echo ""
echo "🛠️ Generating client code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$ECHO_CLIENT_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-echo-client.log"
$ACTR_GEN_CMD install > /dev/null 2>&1 || true
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"

echo -e "${GREEN}✅ actr gen completed (client code refreshed)${NC}"

# Step 1: Build and install actrix
echo ""
echo "📦 Building and installing actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTRIX_CMD=""

if command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
    echo -e "${GREEN}✅ Found actrix in PATH: $(which actrix)${NC}"
elif [ -n "$ACTRIX_DIR" ] && [ -d "$ACTRIX_DIR" ]; then
    echo "actrix not found in PATH, but source directory found at $ACTRIX_DIR."
    echo "Building actrix from source..."
    cd "$ACTRIX_DIR"
    cargo build --features opentelemetry 2>&1 | tail -5
    
    if [ -f "$ACTRIX_DIR/target/debug/actrix" ]; then
        ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
        echo -e "${GREEN}✅ Actrix built successfully: $ACTRIX_CMD${NC}"
    else
        echo -e "${RED}❌ Failed to build actrix from source${NC}"
        exit 1
    fi
    cd "$WORKSPACE_ROOT"
else
    echo -e "${RED}❌ actrix command not found in PATH and source directory not found.${NC}"
    echo "Please install actrix first:"
    echo "  cargo install actrix"
    echo "Or set ACTRIX_DIR to point to the actrix source code directory."
    exit 1
fi

# Step 2: Start actrix
echo ""
echo "🚀 Starting actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

$ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "Actrix started (PID: $ACTRIX_PID)"
echo "Waiting for actrix to be ready..."

# Wait for actrix to start and verify it's listening on port 8081
MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi
    
    # Check if port 8081 is listening (actrix WebSocket server)
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

# Step 2.5a: Setup realms in actrix from actr.toml files
echo ""
echo "🔑 Setting up realms in actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait a bit for supervisord gRPC service to be ready (port 50055)
sleep 2

# Build realm-setup tool if needed
if ! cargo build -p realm-setup 2>&1 | tail -5; then
    echo -e "${RED}❌ Failed to build realm-setup tool${NC}"
    exit 1
fi

# Run realm-setup with actr.toml files from server and client
REALM_SETUP_OUTPUT="$LOG_DIR/realm-setup.log"
if ! cargo run -p realm-setup -- \
    --actrix-config "$ACTRIX_CONFIG" \
    --actr-toml "$ECHO_SERVER_DIR/actr.toml" \
    --actr-toml "$ECHO_CLIENT_DIR/actr.toml" \
    > "$REALM_SETUP_OUTPUT" 2>&1; then
    echo -e "${RED}❌ Failed to setup realms in actrix${NC}"
    cat "$REALM_SETUP_OUTPUT"
    exit 1
fi

echo -e "${GREEN}✅ Realms setup completed${NC}"
cat "$REALM_SETUP_OUTPUT" | grep -E "(Created|Skipped|Found)" || true

# Step 2.5: Build binaries to avoid compilation delay during cargo run
echo ""
echo "🔨 Building binaries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin echo-real-server --bin echo-real-client-app 2>&1; then
    echo -e "${RED}❌ Failed to build binaries${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binaries built successfully${NC}"

# Step 3: Start shell-actr-echo/server
echo ""
echo "🚀 Starting shell-actr-echo/server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-info}" cargo run --bin echo-real-server > "$LOG_DIR/shell-actr-echo-server.log" 2>&1 &
SERVER_PID=$!

echo "Server started (PID: $SERVER_PID)"
echo "Waiting for server to register and connect to signaling server..."

# Wait for server to start and connect
MAX_WAIT=15
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}❌ Server failed to start${NC}"
        cat "$LOG_DIR/shell-actr-echo-server.log"
        exit 1
    fi
    
    # Check if server has successfully connected to signaling server
    if grep -q "ActrNode 启动成功\|Echo Server 已完全启动并注册\|ActrNode started" "$LOG_DIR/shell-actr-echo-server.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Server is running and registered${NC}"
        break
    fi
    
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Server may not have fully registered, but continuing...${NC}"
fi

# Give server a bit more time to fully register
sleep 2

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
    sleep 3
    echo "$TEST_INPUT"
    sleep 2
    echo "quit"
) | RUST_LOG="${RUST_LOG:-info}" cargo run --bin echo-real-client-app > "$LOG_DIR/shell-actr-echo-client.log" 2>&1 &
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
if grep -q "\[Received reply\].*Echo: $TEST_INPUT" "$LOG_DIR/shell-actr-echo-client.log"; then
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
    cat "$LOG_DIR/shell-actr-echo-client.log" | grep "Received reply" || true
    echo ""
    echo "📖 View full logs:"
    echo "   cat $LOG_DIR/shell-actr-echo-client.log  # Client logs"
    echo "   cat $LOG_DIR/shell-actr-echo-server.log  # Server logs"
    echo "   tail -f $LOG_DIR/actrix.log              # Actrix logs"
    echo ""
    exit 0
else
    echo -e "${RED}❌ Test FAILED: Expected server echo response not found${NC}"
    echo -e "${RED}   Looking for: [Received reply] Echo: $TEST_INPUT${NC}"
    echo ""
    echo "Client app output:"
    cat "$LOG_DIR/shell-actr-echo-client.log"
    echo ""
    echo "Server output:"
    cat "$LOG_DIR/shell-actr-echo-server.log" | tail -20
    echo ""
    echo "Actrix output:"
    tail -30 "$LOG_DIR/actrix.log"
    exit 1
fi
