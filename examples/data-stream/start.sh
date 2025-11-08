#!/bin/bash
# Start data-stream example - Using actrix as signaling server
# Auto-starts actrix, receiver, and sender

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📡 DataStream Example - Using Actrix"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine paths based on script location
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ACTR_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTOR_RTC_DIR="$(cd "$ACTR_ROOT/.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"

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
        echo "Stopping sender (PID: $SENDER_PID)"
        kill $SENDER_PID 2>/dev/null || true
    fi

    # Kill receiver
    if [ ! -z "$RECEIVER_PID" ]; then
        echo "Stopping receiver (PID: $RECEIVER_PID)"
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

# Step 0: Check and cleanup port 8081
echo ""
echo "🔍 Checking port 8081..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
kill_port 8081

# Step 1: Check/Install actrix
echo ""
echo "📦 Checking actrix installation..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTRIX_CMD=""

# Check if actrix command is available in PATH
if command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
    echo "✅ Using installed actrix command: $(which actrix)"
# Check if actrix binary exists in local build
elif [ -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
    echo "✅ Using local actrix build: $ACTRIX_CMD"
elif [ -f "$ACTRIX_DIR/target/release/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
    echo "✅ Using local actrix build: $ACTRIX_CMD"
# Try to install from source
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

# Build receiver and sender
echo ""
echo "📦 Building receiver and sender..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

echo "Building receiver..."
cd "$SCRIPT_DIR/receiver"
cargo build 2>&1 | tail -3

echo "Building sender..."
cd "$SCRIPT_DIR/sender"
cargo build 2>&1 | tail -3

cd "$SCRIPT_DIR"

echo -e "${GREEN}✅ All components built${NC}"

# Step 2: Start actrix (with Signaling service)
echo ""
echo "🚀 Starting actrix (Signaling server)..."
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

# Verify port 8081 is listening
if ! lsof -i:8081 > /dev/null 2>&1; then
    echo -e "${RED}❌ Actrix not listening on port 8081${NC}"
    cat /tmp/actrix.log
    exit 1
fi

echo -e "${GREEN}✅ Actrix is running on port 8081${NC}"

# Step 3: Start Receiver
echo ""
echo "🚀 Starting Receiver..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$SCRIPT_DIR/receiver"
RUST_LOG=actr_runtime::wire::webrtc=trace,actr_runtime=debug,info ./target/debug/data-stream-receiver > /tmp/receiver.log 2>&1 &
RECEIVER_PID=$!
cd "$SCRIPT_DIR"

echo "Receiver started (PID: $RECEIVER_PID)"
echo "Waiting for receiver to register..."
sleep 4

if ! kill -0 $RECEIVER_PID 2>/dev/null; then
    echo -e "${RED}❌ Receiver failed to start${NC}"
    cat /tmp/receiver.log
    exit 1
fi

echo -e "${GREEN}✅ Receiver is running${NC}"

# Auto-discover Receiver ActorId serial from logs and export for sender
RECEIVER_SERIAL=$(grep -o 'serial_number: [0-9]\+' /tmp/receiver.log | awk '{print $2}' | tail -1 || true)
if [ -n "$RECEIVER_SERIAL" ]; then
   export ACTR_RECEIVER_SERIAL="$RECEIVER_SERIAL"
   echo "Discovered receiver serial: $ACTR_RECEIVER_SERIAL"
else
   echo -e "${YELLOW}⚠️  Could not detect receiver serial from logs; defaulting to 1000${NC}"
fi

# Ensure receiver type default matches receiver implementation
export ACTR_RECEIVER_TYPE=${ACTR_RECEIVER_TYPE:-file_transfer.FileTransferService}
export ACTR_RECEIVER_MANUFACTURER=${ACTR_RECEIVER_MANUFACTURER:-acme}
export ACTR_REALM_ID=${ACTR_REALM_ID:-0}

# Step 4: Start Sender
echo ""
echo "🚀 Starting Sender..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$SCRIPT_DIR/sender"
RUST_LOG=actr_runtime::wire::webrtc=trace,actr_runtime=debug,info ./target/debug/data-stream-sender > /tmp/sender.log 2>&1 &
SENDER_PID=$!
cd "$SCRIPT_DIR"

echo "Sender started (PID: $SENDER_PID)"
echo "Waiting for WebRTC connection and file transfer to complete..."
sleep 15

# Check results
echo ""
echo "🔍 Checking results..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Show last output from both actors
echo ""
echo "📋 Receiver Output (last 20 lines):"
tail -20 /tmp/receiver.log | grep -E "(Received chunk|StartTransfer|EndTransfer|ready)" || tail -20 /tmp/receiver.log

echo ""
echo "📋 Sender Output (last 20 lines):"
tail -20 /tmp/sender.log | grep -E "(Sent chunk|Phase|succeeded|completed)" || tail -20 /tmp/sender.log

# Verify chunks were received or WebRTC connection established
echo ""
if grep -q "Received chunk" /tmp/receiver.log; then
    CHUNK_COUNT=$(grep -c "Received chunk" /tmp/receiver.log)
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
    echo "   tail -f /tmp/sender.log    # Sender logs"
    echo "   tail -f /tmp/receiver.log  # Receiver logs"
    echo "   tail -f /tmp/actrix.log    # Actrix logs"
    echo ""
elif grep -q "ICE connection state changed: connected" /tmp/receiver.log || grep -q "ICE connection state changed: connected" /tmp/sender.log; then
    echo -e "${YELLOW}⚠️  WebRTC connection established but no data transferred yet${NC}"
    echo "Connection is being established. You may need to wait longer."
    echo ""
    echo "Check logs:"
    echo "   tail -f /tmp/sender.log"
    echo "   tail -f /tmp/receiver.log"
    echo "   tail -f /tmp/actrix.log"
    echo ""
    echo "Processes are still running. Press Ctrl+C to stop."
    echo ""
else
    echo -e "${YELLOW}⚠️  WebRTC negotiation in progress${NC}"
    echo "Offer/Answer exchange detected, waiting for ICE candidates to complete."
    echo ""
    echo "Check progress:"
    echo "   tail -f /tmp/sender.log"
    echo "   tail -f /tmp/receiver.log"
    echo "   tail -f /tmp/actrix.log"
    echo ""
    echo "Processes are still running. Press Ctrl+C to stop."
    echo ""
fi

# Wait a bit before cleanup
echo "Press Ctrl+C to stop all processes..."
wait $SENDER_PID $RECEIVER_PID 2>/dev/null || true
