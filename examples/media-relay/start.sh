#!/bin/bash
# Start media-relay example - Using actrix as signaling server
# Auto-starts actrix, actr-b, and actr-a

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🎬 Media Relay Example - Using Actrix"
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

    # Kill actrix (only if we started it)
    if [ ! -z "$ACTRIX_PID" ] && [ "$ACTRIX_PID" != "external" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    # Kill actr-b
    if [ ! -z "$ACTR_B_PID" ]; then
        echo "Stopping actr-b (PID: $ACTR_B_PID)"
        kill $ACTR_B_PID 2>/dev/null || true
    fi

    # Kill actr-a
    if [ ! -z "$ACTR_A_PID" ]; then
        echo "Stopping actr-a (PID: $ACTR_A_PID)"
        kill $ACTR_A_PID 2>/dev/null || true
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

echo "Building actr-b (receiver)..."
cd "$SCRIPT_DIR/actr-b"
cargo build 2>&1 | tail -3

echo "Building actr-a (relay)..."
cd "$SCRIPT_DIR/actr-a"
cargo build 2>&1 | tail -3

cd "$SCRIPT_DIR"

echo -e "${GREEN}✅ All components built${NC}"

# Step 2: Start actrix (or detect existing one)
echo ""
echo "🚀 Checking actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if port 8081 is already in use
if nc -z localhost 8081 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Port 8081 is already in use${NC}"
    echo -e "${GREEN}✅ Using existing actrix/signaling-server${NC}"
    ACTRIX_PID="external"
else
    echo "Starting actrix..."
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

    echo -e "${GREEN}✅ Actrix is running on port 8081${NC}"
fi

# Step 3: Start Actr B (receiver)
echo ""
echo "🚀 Starting Actr B (Receiver)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$SCRIPT_DIR/actr-b"
./target/debug/actr-b-receiver > /tmp/actr-b.log 2>&1 &
ACTR_B_PID=$!
cd "$SCRIPT_DIR"

echo "Actr B started (PID: $ACTR_B_PID)"
echo "Waiting for Actr B to register..."
sleep 4

if ! kill -0 $ACTR_B_PID 2>/dev/null; then
    echo -e "${RED}❌ Actr B failed to start${NC}"
    cat /tmp/actr-b.log
    exit 1
fi

echo -e "${GREEN}✅ Actr B is running${NC}"

# Step 4: Start Actr A (relay/client)
echo ""
echo "🚀 Starting Actr A (Relay/Client)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$SCRIPT_DIR/actr-a"
./target/debug/actr-a-relay > /tmp/actr-a.log 2>&1 &
ACTR_A_PID=$!
cd "$SCRIPT_DIR"

echo "Actr A started (PID: $ACTR_A_PID)"
echo "Waiting for Actr A to start and send frames..."
sleep 12

# Check if Actr A is still running (it should complete after sending frames)
echo ""
echo "🔍 Checking results..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Show last output from both actors
echo ""
echo "📋 Actr B Output (last 15 lines):"
tail -15 /tmp/actr-b.log | grep -E "(Received frame|启动|成功|注册)" || tail -15 /tmp/actr-b.log

echo ""
echo "📋 Actr A Output (last 20 lines):"
tail -20 /tmp/actr-a.log | grep -E "(生成帧|已发送|完成|启动|成功)" || tail -20 /tmp/actr-a.log

# Verify frames were received
echo ""
if grep -q "Received frame" /tmp/actr-b.log; then
    FRAME_COUNT=$(grep -c "Received frame" /tmp/actr-b.log)
    echo -e "${GREEN}✅ Test PASSED: Actr B received $FRAME_COUNT frames${NC}"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🎉 Demo completed successfully!"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "✅ Validated:"
    echo "   • Real ActrSystem lifecycle"
    echo "   • Real WebRTC P2P connection establishment"
    echo "   • Real RPC message routing and dispatch"
    echo "   • Real protobuf encode/decode"
    echo "   • Real distributed Actor communication"
    echo "   • Using actrix as signaling server"
    echo ""
    echo "📖 View full logs:"
    echo "   tail -f /tmp/actr-a.log    # Sender logs"
    echo "   tail -f /tmp/actr-b.log    # Receiver logs"
    if [ "$ACTRIX_PID" != "external" ]; then
        echo "   tail -f /tmp/actrix.log  # Actrix logs"
    fi
    echo ""
else
    echo -e "${RED}❌ Test FAILED: No frames received${NC}"
    echo ""
    echo "Full logs:"
    echo "=== Actr B ==="
    cat /tmp/actr-b.log
    echo ""
    echo "=== Actr A ==="
    cat /tmp/actr-a.log
    if [ "$ACTRIX_PID" != "external" ]; then
        echo ""
        echo "=== Actrix ==="
        tail -50 /tmp/actrix.log
    fi
    exit 1
fi

# Wait a bit before cleanup
echo "Press Ctrl+C to stop all processes..."
wait $ACTR_A_PID $ACTR_B_PID 2>/dev/null || true
