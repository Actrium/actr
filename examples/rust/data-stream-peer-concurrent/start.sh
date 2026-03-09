#!/bin/bash
# Start data-stream-peer-concurrent example - Bidirectional peer-to-peer streaming with concurrent clients
# Auto-starts actrix, server, and multiple clients to demonstrate concurrency

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

show_usage() {
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}📡 Data Stream Peer Concurrent Example${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "${GREEN}Usage:${NC}"
    echo "  $0 [OPTIONS]"
    echo ""
    echo -e "${GREEN}Options:${NC}"
    echo "  -c, --count <number>    Number of messages each client should receive (default: 5)"
    echo "  -n, --num-clients <n>   Number of concurrent clients to start (default: 3)"
    echo "  -h, --help              Show this help message"
    echo ""
    echo -e "${GREEN}Examples:${NC}"
    echo "  $0                      # 3 clients, 5 messages each"
    echo "  $0 -c 10                # 3 clients, 10 messages each"
    echo "  $0 -c 5 -n 5            # 5 clients, 5 messages each"
    echo ""
    exit 0
}

MESSAGE_COUNT=5
NUM_CLIENTS=3

while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            show_usage
            ;;
        -c|--count)
            MESSAGE_COUNT="$2"
            shift 2
            ;;
        -n|--num-clients)
            NUM_CLIENTS="$2"
            shift 2
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            show_usage
            exit 1
            ;;
    esac
done

if ! [[ "$MESSAGE_COUNT" =~ ^[0-9]+$ ]] || [ "$MESSAGE_COUNT" -lt 1 ]; then
    echo -e "${RED}Error: MESSAGE_COUNT must be a positive integer${NC}"
    exit 1
fi

if ! [[ "$NUM_CLIENTS" =~ ^[0-9]+$ ]] || [ "$NUM_CLIENTS" -lt 1 ]; then
    echo -e "${RED}Error: NUM_CLIENTS must be a positive integer${NC}"
    exit 1
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📡 Data Stream Peer Concurrent Example"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo -e "${GREEN}Configuration:${NC}"
echo "   Message count per client: $MESSAGE_COUNT"
echo "   Number of concurrent clients: $NUM_CLIENTS"
echo ""

# Determine paths and switch to workspace root
WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROTO_DIR="$SCRIPT_DIR/proto"
SHARED_DIR="$SCRIPT_DIR/shared"

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
ensure_actr_toml "$SCRIPT_DIR/server"
ensure_actr_toml "$SCRIPT_DIR/client"

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

    # Kill all clients
    for pid in "${CLIENT_PIDS[@]}"; do
        if [ ! -z "$pid" ]; then
            echo "Stopping client (PID: $pid)"
            kill $pid 2>/dev/null || true
        fi
    done

    # Kill server
    if [ ! -z "$SERVER_PID" ]; then
        echo "Stopping server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
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
echo "🔧 Step 0: Generating code with actr gen..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Generate shared code
echo "Generating shared code..."
cd "$SHARED_DIR"
actr gen --input=../proto --output=src/generated --clean > "$LOG_DIR/actr-gen-peer-concurrent.log" 2>&1
if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ Shared code generated${NC}"
else
    echo -e "${RED}❌ Code generation failed${NC}"
    cat "$LOG_DIR/actr-gen-peer-concurrent.log"
    exit 1
fi

# Return to workspace root
cd "$WORKSPACE_ROOT"

# Step 1: Build actrix (signaling server)
echo ""
echo "🏗️  Step 1: Building actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$ACTRIX_DIR"
cargo build --release --bin actrix
if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ Actrix built successfully${NC}"
else
    echo -e "${RED}❌ Actrix build failed${NC}"
    exit 1
fi

# Return to workspace root
cd "$WORKSPACE_ROOT"

# Step 2: Build server and client
echo ""
echo "🏗️  Step 2: Building server and client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Build server
echo "Building server..."
cargo build --bin data-stream-peer-concurrent-server
if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ Server built successfully${NC}"
else
    echo -e "${RED}❌ Server build failed${NC}"
    exit 1
fi

# Build client
echo "Building client..."
cargo build --bin data-stream-peer-concurrent-client
if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ Client built successfully${NC}"
else
    echo -e "${RED}❌ Client build failed${NC}"
    exit 1
fi

# Step 3: Start actrix (signaling server)
echo ""
echo "🚀 Step 3: Starting actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if actrix is already running
if lsof -ti:8081 >/dev/null 2>&1; then
    echo -e "${YELLOW}⚠️  Port 8081 already in use, attempting to kill existing process${NC}"
    kill_port 8081
fi

# Start actrix in background
ACTRIX_BIN="$ACTRIX_DIR/target/release/actrix"
echo "Starting: $ACTRIX_BIN --config $ACTRIX_CONFIG"
$ACTRIX_BIN --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!
echo "$ACTRIX_PID" > "$LOG_DIR/actrix.pid"
echo -e "${GREEN}✅ Actrix started (PID: $ACTRIX_PID)${NC}"
echo "   Log: $LOG_DIR/actrix.log"

# Wait for actrix to be ready
echo "⏳ Waiting for actrix to start..."
sleep 3

# Check if actrix is still running
if ! kill -0 $ACTRIX_PID 2>/dev/null; then
    echo -e "${RED}❌ Actrix failed to start. Check logs:${NC}"
    tail -20 "$LOG_DIR/actrix.log"
    exit 1
fi
echo -e "${GREEN}✅ Actrix is running${NC}"

# Step 4: Start server
echo ""
echo "🚀 Step 4: Starting server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG=info cargo run --bin data-stream-peer-concurrent-server > "$LOG_DIR/data-stream-peer-concurrent-server.log" 2>&1 &
SERVER_PID=$!
echo -e "${GREEN}✅ Server started (PID: $SERVER_PID)${NC}"
echo "   Log: $LOG_DIR/data-stream-peer-concurrent-server.log"

# Wait for server to initialize and register with signaling server
echo "⏳ Waiting for server to initialize and register..."
sleep 10

# Check if server is still running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo -e "${RED}❌ Server failed to start. Check logs:${NC}"
    tail -20 "$LOG_DIR/data-stream-peer-concurrent-server.log"
    exit 1
fi
echo -e "${GREEN}✅ Server is ready${NC}"

# Step 5: Start multiple clients to demonstrate concurrency
echo ""
echo "🚀 Step 5: Starting multiple clients (concurrency test)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Array to track client PIDs
declare -a CLIENT_PIDS

# Concurrently start all clients
echo "🚀 Starting $NUM_CLIENTS clients concurrently..."

for i in $(seq 1 $NUM_CLIENTS); do
    CLIENT_ID="client-$i"
    RUST_LOG=info cargo run --bin data-stream-peer-concurrent-client -- "$CLIENT_ID" "$MESSAGE_COUNT" > "$LOG_DIR/data-stream-peer-concurrent-client-$i.log" 2>&1 &
    CLIENT_PID=$!
    CLIENT_PIDS+=($CLIENT_PID)
    echo -e "${GREEN}✅ Client #$i started (PID: $CLIENT_PID, ID: $CLIENT_ID)${NC}"
    echo "   Log: $LOG_DIR/data-stream-peer-concurrent-client-$i.log"
    sleep 1
done

echo ""
echo "⏳ All clients started, waiting for connections to establish..."
sleep 2

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "${GREEN}✅ All processes started successfully${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "📊 Process Status:"
echo "   Actrix:  PID $ACTRIX_PID (log: logs/actrix.log)"
echo "   Server:  PID $SERVER_PID (log: logs/data-stream-peer-concurrent-server.log)"
for i in $(seq 1 $NUM_CLIENTS); do
    echo "   Client #$i: PID ${CLIENT_PIDS[$((i-1))]} (log: logs/data-stream-peer-concurrent-client-$i.log)"
done
echo ""
echo "💡 To view logs in real-time:"
echo "   tail -f logs/data-stream-peer-concurrent-server.log"
echo "   tail -f logs/data-stream-peer-concurrent-client-1.log"
echo ""
echo "Press Ctrl+C to stop all processes"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait for all clients to complete
wait "${CLIENT_PIDS[@]}"

echo ""
echo "🎉 All clients completed their streaming!"
echo "📊 Check logs for detailed streaming statistics"
echo ""
echo "Press Ctrl+C to stop server and actrix, or they will continue running..."

# Keep server and actrix running
wait
