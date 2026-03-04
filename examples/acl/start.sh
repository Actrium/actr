#!/usr/bin/env bash
set -euo pipefail

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Directories
WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
ACTRIX_CONFIG="$WORKSPACE_ROOT/acl/actrix-config.toml"
ACL_DEMO_DIR="$WORKSPACE_ROOT/acl"
SERVER_DIR="$ACL_DEMO_DIR/server"
ALLOWED_CLIENT_DIR="$ACL_DEMO_DIR/allowed-client"
BLOCKED_CLIENT_DIR="$ACL_DEMO_DIR/blocked-client"
PROTO_DIR="$ACL_DEMO_DIR/proto"

cd "$WORKSPACE_ROOT"

LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

echo ""
echo "🔍 Checking Actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$SERVER_DIR"
ensure_actr_toml "$ALLOWED_CLIENT_DIR"
ensure_actr_toml "$BLOCKED_CLIENT_DIR"
ensure_actrix_config "$WORKSPACE_ROOT"

echo ""
echo "🧰 Checking required CLI tools..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_cargo_bin "protoc-gen-prost" "protoc-gen-prost" "$LOG_DIR"
ensure_cargo_bin "protoc-gen-tonic" "protoc-gen-tonic" "$LOG_DIR"

# Step 1: Build and start actrix
echo ""
echo "🚀 Step 1: Building and starting actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if actrix directory exists
if [ ! -d "$ACTRIX_DIR" ]; then
    echo -e "${RED}❌ Cannot find actrix directory at $ACTRIX_DIR${NC}"
    exit 1
fi

# Build actrix
echo "Building actrix from source..."
cd "$ACTRIX_DIR"
if ! cargo build 2>&1 | tee "$LOG_DIR/actrix-build.log" | tail -5; then
    echo -e "${RED}❌ Failed to build actrix${NC}"
    exit 1
fi

# Check if build was successful
if [ ! -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    echo -e "${RED}❌ Failed to build actrix${NC}"
    exit 1
fi

cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Actrix built successfully${NC}"

# Start actrix
echo "Starting actrix..."
"$ACTRIX_DIR/target/debug/actrix" --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!
echo "Actrix started (PID: $ACTRIX_PID)"

# Wait for actrix to be ready
MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi
    if curl -s http://127.0.0.1:7000/health > /dev/null 2>&1; then
        echo -e "${GREEN}✅ Actrix is running${NC}"
        break
    fi
    sleep 1
    COUNTER=$((COUNTER+1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Actrix health check timeout, proceeding anyway...${NC}"
fi

function cleanup {
    echo ""
    echo "🧹 Cleaning up..."
    [ -n "${SERVER_PID:-}" ] && kill $SERVER_PID 2>/dev/null || true
    [ -n "${ALLOWED_CLIENT_PID:-}" ] && kill $ALLOWED_CLIENT_PID 2>/dev/null || true
    [ -n "${BLOCKED_CLIENT_PID:-}" ] && kill $BLOCKED_CLIENT_PID 2>/dev/null || true
    [ -n "${ACTRIX_PID:-}" ] && kill $ACTRIX_PID 2>/dev/null || true
}
trap cleanup EXIT

# Step 2: Generate code
echo ""
echo "🛠️  Generating code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_GEN_CMD=""
if command -v actr > /dev/null 2>&1; then
    ACTR_GEN_CMD="actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/debug/actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/release/actr"
else
    echo -e "${RED}❌ actr generator not found${NC}"
    exit 1
fi

if [ ! -d "$PROTO_DIR" ]; then
    echo -e "${RED}❌ Proto directory not found at $PROTO_DIR${NC}"
    exit 1
fi

cd "$ACL_DEMO_DIR"
if ! $ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$LOG_DIR/actr-gen.log" 2>&1; then
    echo -e "${RED}❌ Code generation failed${NC}"
    cat "$LOG_DIR/actr-gen.log"
    exit 1
fi
echo -e "${GREEN}✅ Code generated successfully${NC}"

# Step 3: Build binaries
echo ""
echo "🔨 Building binaries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$WORKSPACE_ROOT"
if ! cargo build --bin acl-server --bin allowed-client --bin blocked-client > "$LOG_DIR/cargo-build.log" 2>&1; then
    echo -e "${RED}❌ Build failed${NC}"
    tail -20 "$LOG_DIR/cargo-build.log"
    exit 1
fi
echo -e "${GREEN}✅ Binaries built successfully${NC}"

# Step 4: Start server
echo ""
echo "🚀 Starting ACL-protected server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$SERVER_DIR"
RUST_LOG="${RUST_LOG:-info}" "$WORKSPACE_ROOT/target/debug/acl-server" > "$LOG_DIR/acl-server.log" 2>&1 &
SERVER_PID=$!
echo "Server started (PID: $SERVER_PID)"

MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}❌ Server failed to start${NC}"
        cat "$LOG_DIR/acl-server.log"
        exit 1
    fi
    if grep -q "ActrNode started\|Greeter Server is running" "$LOG_DIR/acl-server.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Server is running${NC}"
        break
    fi
    sleep 1
    COUNTER=$((COUNTER + 1))
done

sleep 1

# Step 5: Test allowed client
echo ""
echo "🧪 Testing ALLOWED client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$ALLOWED_CLIENT_DIR"
RUST_LOG="${RUST_LOG:-info}" "$WORKSPACE_ROOT/target/debug/allowed-client" > "$LOG_DIR/allowed-client.log" 2>&1 &
ALLOWED_CLIENT_PID=$!

sleep 3
wait $ALLOWED_CLIENT_PID || true

# Step 6: Test blocked client
echo ""
echo "🧪 Testing BLOCKED client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$BLOCKED_CLIENT_DIR"
RUST_LOG="${RUST_LOG:-info}" "$WORKSPACE_ROOT/target/debug/blocked-client" > "$LOG_DIR/blocked-client.log" 2>&1 &
BLOCKED_CLIENT_PID=$!

sleep 3
wait $BLOCKED_CLIENT_PID || true

# Step 7: Verify results
echo ""
echo "📊 Verifying ACL test results..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ALLOWED_SUCCESS=false
BLOCKED_FAILED=false

if grep -q "ACL test PASSED - Allowed client can access server" "$LOG_DIR/allowed-client.log"; then
    ALLOWED_SUCCESS=true
    echo -e "${GREEN}✅ Allowed client test PASSED${NC}"
else
    echo -e "${RED}❌ Allowed client test FAILED${NC}"
fi

if grep -q "ACL test PASSED - Blocked client was correctly denied" "$LOG_DIR/blocked-client.log"; then
    BLOCKED_FAILED=true
    echo -e "${GREEN}✅ Blocked client test PASSED${NC}"
else
    echo -e "${RED}❌ Blocked client test FAILED${NC}"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ "$ALLOWED_SUCCESS" = true ] && [ "$BLOCKED_FAILED" = true ]; then
    echo -e "${GREEN}🎉 ACL Demo Test PASSED!${NC}"
    echo ""
    echo "✅ Validated:"
    echo "   • ACL rules correctly allow 'allowed-greeter-client'"
    echo "   • ACL rules correctly block 'blocked-greeter-client'"
    echo "   • ACL configuration from Actr.toml works"
    echo "   • Real distributed Actor communication with ACL"
    echo ""
else
    echo -e "${RED}❌ ACL Demo Test FAILED${NC}"
    echo ""
fi

echo "📖 View full logs:"
echo "   cat $LOG_DIR/acl-server.log         # Server logs"
echo "   cat $LOG_DIR/allowed-client.log     # Allowed client logs"
echo "   cat $LOG_DIR/blocked-client.log     # Blocked client logs"
echo "   tail -f $LOG_DIR/actrix.log         # Actrix logs"
echo ""

if [ "$ALLOWED_SUCCESS" = true ] && [ "$BLOCKED_FAILED" = true ]; then
    exit 0
else
    exit 1
fi
