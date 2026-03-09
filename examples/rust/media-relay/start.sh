#!/bin/bash
# Start media-relay example - Using actrix as signaling server
# Auto-starts actrix, actr-b, and actr-a

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🎬 Media Relay Example - Using Actrix"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine paths and switch to workspace root
WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/.." && pwd)"
ACTR_DIR="$ACTOR_RTC_DIR/actr"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
PROTO_DIR="$WORKSPACE_ROOT/media-relay/proto"

# Switch to workspace root and stay there
cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required CLI tools
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"

# Ensure actr.toml files exist
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# Ensure actr.toml files exist for actr-a and actr-b
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ACTR_B_DIR="$WORKSPACE_ROOT/media-relay/actr-b"
ACTR_A_DIR="$WORKSPACE_ROOT/media-relay/actr-a"
ensure_actr_toml "$ACTR_B_DIR"
ensure_actr_toml "$ACTR_A_DIR"

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

# Step 0: Generate code for actr-a and actr-b
echo ""
echo "🛠️ Generating code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_GEN_CMD=""

if command -v actr > /dev/null 2>&1; then
    ACTR_GEN_CMD="actr"
elif [ -x "$ACTR_DIR/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTR_DIR/target/debug/actr"
elif [ -x "$ACTR_DIR/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTR_DIR/target/release/actr"
else
    echo -e "${RED}❌ actr generator not found (expected 'actr' in PATH or built under $ACTR_DIR)${NC}"
    exit 1
fi

if [ ! -d "$PROTO_DIR" ]; then
    echo -e "${RED}❌ Proto directory not found at $PROTO_DIR${NC}"
    exit 1
fi

for crate in actr-b actr-a; do
    CRATE_DIR="$WORKSPACE_ROOT/media-relay/$crate"
    OUTPUT_FILE="$LOG_DIR/actr-gen-$crate.log"
    echo "🔧 Running actr gen for $crate..."
    cd "$CRATE_DIR"
    GEN_CMD=("$ACTR_GEN_CMD" gen --input="$PROTO_DIR" --output=src/generated --clean)
    if [ "$crate" = "actr-a" ]; then
        GEN_CMD+=("--no-scaffold")
    fi
    "${GEN_CMD[@]}" > "$OUTPUT_FILE" 2>&1 || {
        echo -e "${RED}❌ actr gen failed for $crate${NC}"
        cat "$OUTPUT_FILE"
        exit 1
    }
done

cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ actr gen completed for actr-a and actr-b${NC}"

# Step 1: Check/Build actrix
echo ""
echo "📦 Checking actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

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
    cd "$WORKSPACE_ROOT"
else
    echo -e "${RED}❌ Cannot find actrix directory at $ACTRIX_DIR${NC}"
    echo "Please ensure actrix project exists at: $ACTRIX_DIR"
    exit 1
fi

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
    $ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
    ACTRIX_PID=$!

    echo "Actrix started (PID: $ACTRIX_PID)"
    echo "Waiting for actrix to be ready..."
    sleep 3

    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi

    echo -e "${GREEN}✅ Actrix is running on port 8081${NC}"
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

# Run realm-setup with actr.toml files from actr-a and actr-b
REALM_SETUP_OUTPUT="$LOG_DIR/realm-setup.log"
if ! cargo run -p realm-setup -- \
    --actrix-config "$ACTRIX_CONFIG" \
    --actr-toml "$ACTR_A_DIR/actr.toml" \
    --actr-toml "$ACTR_B_DIR/actr.toml" \
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

if ! cargo build --bin actr-b-receiver --bin actr-a-relay 2>&1; then
    echo -e "${RED}❌ Failed to build binaries${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binaries built successfully${NC}"

# Step 3: Start Actr B (receiver)
echo ""
echo "🚀 Starting Actr B (Receiver)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-info}" cargo run --bin actr-b-receiver > "$LOG_DIR/actr-b.log" 2>&1 &
ACTR_B_PID=$!

echo "Actr B started (PID: $ACTR_B_PID)"
echo "Waiting for Actr B to register..."
MAX_WAIT=25
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTR_B_PID 2>/dev/null; then
        echo -e "${RED}❌ Actr B failed to start${NC}"
        cat "$LOG_DIR/actr-b.log"
        exit 1
    fi

    if grep -q "Actr B 已完全启动并注册\|ActrNode 启动成功" "$LOG_DIR/actr-b.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Actr B is running${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Actr B may still be registering, continuing...${NC}"
fi

# Step 4: Start Actr A (relay/client)
echo ""
echo "🚀 Starting Actr A (Relay/Client)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-info}" cargo run --bin actr-a-relay > "$LOG_DIR/actr-a.log" 2>&1 &
ACTR_A_PID=$!

echo "Actr A started (PID: $ACTR_A_PID)"
echo "Waiting for Actr A to start and send frames..."
MAX_WAIT=25
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTR_A_PID 2>/dev/null; then
        echo -e "${RED}❌ Actr A failed to start${NC}"
        cat "$LOG_DIR/actr-a.log"
        exit 1
    fi

    if grep -q "ActrNode 启动成功" "$LOG_DIR/actr-a.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Actr A is running${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Actr A may still be compiling or starting, continuing...${NC}"
fi

echo ""
echo "⏳ Waiting for media frames to be sent and received..."
FRAME_WAIT=30
FRAME_COUNTER=0
while [ $FRAME_COUNTER -lt $FRAME_WAIT ]; do
    if grep -q "Received frame" "$LOG_DIR/actr-b.log" 2>/dev/null; then
        break
    fi

    sleep 1
    FRAME_COUNTER=$((FRAME_COUNTER + 1))
done

# Check if Actr A is still running (it should complete after sending frames)
echo ""
echo "🔍 Checking results..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Show last output from both actors
echo ""
echo "📋 Actr B Output (last 15 lines):"
tail -15 "$LOG_DIR/actr-b.log" | grep -E "(Received frame|启动|成功|注册)" || tail -15 "$LOG_DIR/actr-b.log"

echo ""
echo "📋 Actr A Output (last 20 lines):"
tail -20 "$LOG_DIR/actr-a.log" | grep -E "(生成帧|已发送|完成|启动|成功)" || tail -20 "$LOG_DIR/actr-a.log"

# Verify frames were received
echo ""
if grep -q "Received frame" "$LOG_DIR/actr-b.log"; then
    FRAME_COUNT=$(grep -c "Received frame" "$LOG_DIR/actr-b.log")
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
    echo "   tail -f $LOG_DIR/actr-a.log    # Sender logs"
    echo "   tail -f $LOG_DIR/actr-b.log    # Receiver logs"
    if [ "$ACTRIX_PID" != "external" ]; then
        echo "   tail -f $LOG_DIR/actrix.log  # Actrix logs"
    fi
    echo ""
    exit 0
else
    echo -e "${RED}❌ Test FAILED: No frames received${NC}"
    echo ""
    echo "Full logs:"
    echo "=== Actr B ==="
    cat "$LOG_DIR/actr-b.log"
    echo ""
    echo "=== Actr A ==="
    cat "$LOG_DIR/actr-a.log"
    if [ "$ACTRIX_PID" != "external" ]; then
        echo ""
        echo "=== Actrix ==="
        tail -50 "$LOG_DIR/actrix.log"
    fi
    exit 1
fi

# Wait a bit before cleanup
echo "Press Ctrl+C to stop all processes..."
wait $ACTR_A_PID $ACTR_B_PID 2>/dev/null || true
