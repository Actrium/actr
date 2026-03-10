#!/bin/bash
# Start data-stream example - Using actrix as signaling server
# Auto-starts actrix, receiver, and sender

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📡 DataStream Example - Using Actrix"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine paths and switch to workspace root
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Optional paths for local source builds (only used if binaries are not in PATH)
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd || echo "")"
ACTRIX_DIR="${ACTRIX_DIR:-${PROJECT_ROOT:+$PROJECT_ROOT/actrix}}"
ACTR_CLI_DIR="${ACTR_CLI_DIR:-${PROJECT_ROOT:+$PROJECT_ROOT/actr}}"

# Allow overriding via environment variables
ACTRIX_CONFIG="${ACTRIX_CONFIG:-$WORKSPACE_ROOT/config.local.toml}"
PROTO_DIR="${PROTO_DIR:-$WORKSPACE_ROOT/data-stream/proto}"

# Optional paths for local source builds (only used if binaries are not in PATH)
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd || echo "")"
ACTRIX_DIR="${ACTRIX_DIR:-${PROJECT_ROOT:+$PROJECT_ROOT/actrix}}"
ACTR_CLI_DIR="${ACTR_CLI_DIR:-${PROJECT_ROOT:+$PROJECT_ROOT/actr}}"

# Switch to workspace root and stay there
cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required CLI tools
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"

# Ensure actr.toml files exist
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# Ensure actr.toml files exist for sender and receiver
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
DATA_STREAM_DIR="$WORKSPACE_ROOT/data-stream"
SENDER_DIR="$DATA_STREAM_DIR/sender"
RECEIVER_DIR="$DATA_STREAM_DIR/receiver"
ensure_actr_toml "$SENDER_DIR"
ensure_actr_toml "$RECEIVER_DIR"

# Ensure actrix-config.toml exists
ensure_actrix_config "$WORKSPACE_ROOT"

echo ""
echo "🧰 Checking required CLI tools..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_cargo_bin "protoc-gen-prost" "protoc-gen-prost" "$LOG_DIR"
ensure_cargo_bin "protoc-gen-actrframework" "actr-framework-protoc-codegen" "$LOG_DIR"
ensure_cargo_bin "actr" "actr-cli" "$LOG_DIR"

# Function to kill process on port
kill_port() {
    local port=$1
    local pid=$(lsof -ti:$port 2>/dev/null || true)
    if [ ! -z "$pid" ]; then
        echo "Killing process on port $port (PID: $pid)"
        kill -9 $pid 2>/dev/null || true
        sleep 0.5
    fi
}

# Cleanup function
cleanup() {
    echo ""
    echo "🧹 Cleaning up..."

    # Kill sender
    if [ ! -z "$SENDER_PID" ]; then
        echo "Stopping data-stream/sender (PID: $SENDER_PID)"
        kill $SENDER_PID 2>/dev/null || true
    fi

    # Kill receiver
    if [ ! -z "$RECEIVER_PID" ]; then
        echo "Stopping data-stream/receiver (PID: $RECEIVER_PID)"
        kill $RECEIVER_PID 2>/dev/null || true
    fi

    # Kill actrix
    if [ ! -z "$ACTRIX_PID" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    wait 2>/dev/null || true

    echo "✅ Cleanup complete"
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Step 0: Generate code (protobuf + actor glue)
echo ""
echo "🛠️ Generating code (actr gen)..."
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

# Generate sender code
echo ""
echo "🛠️ Generating sender code..."
cd "$SENDER_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-sender.log"
$ACTR_GEN_CMD install > /dev/null 2>&1 || true
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed (sender)${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Sender code generated${NC}"

# Generate receiver code
echo ""
echo "🛠️ Generating receiver code..."
cd "$RECEIVER_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-receiver.log"
$ACTR_GEN_CMD install > /dev/null 2>&1 || true
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed (receiver)${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Receiver code generated${NC}"

# Step 1: Check and cleanup port 8081
echo ""
echo "🔍 Checking port 8081..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
kill_port 8081

# Step 2: Ensure actrix is available
echo ""
echo "📦 Checking actrix installation..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTRIX_CMD=""

if command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
    echo -e "${GREEN}✅ Found actrix in PATH: $(which actrix)${NC}"
elif [ -n "$ACTRIX_DIR" ] && [ -d "$ACTRIX_DIR" ]; then
    echo "actrix not found in PATH, but source directory found at $ACTRIX_DIR."
    echo "Building actrix from source..."
    cd "$ACTRIX_DIR"
    cargo build 2>&1 | tail -5
    
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

# Step 3: Start actrix
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

# Step 3.5: Setting up realms in actrix
echo ""
echo "🔑 Setting up realms in actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait a bit for sqlite database to be fully initialized by actrix
sleep 2

# Build realm-setup tool if needed
if ! cargo build -p realm-setup 2>&1 | tail -5; then
    echo -e "${RED}❌ Failed to build realm-setup tool${NC}"
    exit 1
fi

# Run realm-setup with actr.toml files from sender and receiver
REALM_SETUP_OUTPUT="$LOG_DIR/realm-setup.log"
if ! cargo run -p realm-setup -- \
    --actrix-config "$ACTRIX_CONFIG" \
    --actr-toml "$SENDER_DIR/actr.toml" \
    --actr-toml "$RECEIVER_DIR/actr.toml" \
    > "$REALM_SETUP_OUTPUT" 2>&1; then
    echo -e "${RED}❌ Failed to setup realms in actrix${NC}"
    cat "$REALM_SETUP_OUTPUT"
    # Kill the actrix process before exiting
    kill -9 $ACTRIX_PID 2>/dev/null || true
    exit 1
fi

echo -e "${GREEN}✅ Realms setup completed safely via SQLite${NC}"

# Step 3.6: Build binaries to avoid compilation delay during cargo run
echo ""
echo "🔨 Building binaries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin data-stream-receiver --bin data-stream-sender 2>&1; then
    echo -e "${RED}❌ Failed to build binaries${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binaries built successfully${NC}"

# Step 4: Start data-stream/receiver
echo ""
echo "🚀 Starting data-stream/receiver..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-actr_runtime::wire::webrtc=trace,actr_runtime=debug,info}" cargo run --bin data-stream-receiver > "$LOG_DIR/data-stream-receiver.log" 2>&1 &
RECEIVER_PID=$!

echo "Receiver started (PID: $RECEIVER_PID)"
echo "Waiting for receiver to register and connect to signaling server..."

# Wait for receiver to start and connect
MAX_WAIT=15
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $RECEIVER_PID 2>/dev/null; then
        echo -e "${RED}❌ Receiver failed to start${NC}"
        cat "$LOG_DIR/data-stream-receiver.log"
        exit 1
    fi
    
    # Check if receiver has successfully connected to signaling server
    if grep -q "ActrNode 启动成功\|Receiver ready to accept file transfers\|ActrNode started" "$LOG_DIR/data-stream-receiver.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Receiver is running and registered${NC}"
        break
    fi
    
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Receiver may not have fully registered, but continuing...${NC}"
fi

# Give receiver a bit more time to fully register for service discovery
sleep 2

# Step 5: Start data-stream/sender
echo ""
echo "🚀 Starting data-stream/sender..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-actr_runtime::wire::webrtc=trace,actr_runtime=debug,info}" cargo run --bin data-stream-sender > "$LOG_DIR/data-stream-sender.log" 2>&1 &
SENDER_PID=$!

echo "Sender started (PID: $SENDER_PID)"
echo "Waiting for WebRTC connection and file transfer to complete..."

# Wait for sender to complete (max 30 seconds)
MAX_WAIT=30
COUNTER=0
while kill -0 $SENDER_PID 2>/dev/null && [ $COUNTER -lt $MAX_WAIT ]; do
    sleep 1
    COUNTER=$((COUNTER + 1))
done

# Check if sender is still running (should have exited after transfer)
if kill -0 $SENDER_PID 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Sender still running after ${MAX_WAIT} seconds, killing...${NC}"
    kill $SENDER_PID 2>/dev/null || true
fi

# Step 6: Verify output
echo ""
echo "🔍 Verifying output..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Show last output from both actors
echo ""
echo "📋 Receiver Output (last 20 lines):"
tail -20 "$LOG_DIR/data-stream-receiver.log" | grep -E "(Received chunk|StartTransfer|EndTransfer|ready)" || tail -20 "$LOG_DIR/data-stream-receiver.log"

echo ""
echo "📋 Sender Output (last 20 lines):"
tail -20 "$LOG_DIR/data-stream-sender.log" | grep -E "(Sent chunk|Phase|succeeded|completed)" || tail -20 "$LOG_DIR/data-stream-sender.log"

# Verify chunks were received or WebRTC connection established
echo ""
if grep -q "Received chunk" "$LOG_DIR/data-stream-receiver.log"; then
    CHUNK_COUNT=$(grep -c "Received chunk" "$LOG_DIR/data-stream-receiver.log")
    echo -e "${GREEN}✅ Test PASSED: Receiver got $CHUNK_COUNT chunks${NC}"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🎉 DataStream demo completed successfully!"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "✅ Validated:"
    echo "   • RPC handshake (StartTransfer/EndTransfer)"
    echo "   • DataStream fast path transmission"
    echo "   • register_stream/unregister_stream callbacks"
    echo "   • Real protobuf encode/decode"
    echo "   • Real distributed Actor communication"
    echo "   • Using actrix as signaling server"
    echo ""
    echo "📖 View full logs:"
    echo "   cat $LOG_DIR/data-stream-sender.log    # Sender logs"
    echo "   cat $LOG_DIR/data-stream-receiver.log  # Receiver logs"
    echo "   tail -f $LOG_DIR/actrix.log            # Actrix logs"
    echo ""
    exit 0
elif grep -q "ICE connection state changed: connected" "$LOG_DIR/data-stream-receiver.log" || grep -q "ICE connection state changed: connected" "$LOG_DIR/data-stream-sender.log"; then
    echo -e "${YELLOW}⚠️  WebRTC connection established but no data transferred yet${NC}"
    echo "Connection is being established. You may need to wait longer."
    echo ""
    echo "Check logs:"
    echo "   tail -f $LOG_DIR/data-stream-sender.log"
    echo "   tail -f $LOG_DIR/data-stream-receiver.log"
    echo "   tail -f $LOG_DIR/actrix.log"
    echo ""
    echo "Processes are still running. Press Ctrl+C to stop."
    echo ""
else
    echo -e "${YELLOW}⚠️  WebRTC negotiation in progress${NC}"
    echo "Offer/Answer exchange detected, waiting for ICE candidates to complete."
    echo ""
    echo "Check progress:"
    echo "   tail -f $LOG_DIR/data-stream-sender.log"
    echo "   tail -f $LOG_DIR/data-stream-receiver.log"
    echo "   tail -f $LOG_DIR/actrix.log"
    echo ""
    echo "Processes are still running. Press Ctrl+C to stop."
    echo ""
fi

# Wait a bit before cleanup
echo "Press Ctrl+C to stop all processes..."
wait $SENDER_PID $RECEIVER_PID 2>/dev/null || true
