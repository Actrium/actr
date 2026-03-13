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
# Use BASH_SOURCE[0] to reliably locate this script regardless of CWD
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# SCRIPT_DIR       = .../Actrium/actr/examples/rust/shell-actr-echo
# WORKSPACE_ROOT   = .../Actrium/actr/examples/rust  (the Cargo workspace)
# ACTOR_RTC_DIR    = .../Actrium                     (repo root, parent of both actr/ and actrix/)
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
ECHO_SERVER_DIR="$SCRIPT_DIR/server"
ECHO_CLIENT_DIR="$SCRIPT_DIR/client"
PROTO_DIR="$SCRIPT_DIR/proto"

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
# Priority: WORKSPACE_ROOT/actrix-config.toml > WORKSPACE_ROOT/actrix-config.example.toml > ACTRIX_DIR/config.example.toml
if [ ! -f "$ACTRIX_CONFIG" ]; then
    if [ -f "$WORKSPACE_ROOT/actrix-config.example.toml" ]; then
        echo "📋 Copying $WORKSPACE_ROOT/actrix-config.example.toml to $ACTRIX_CONFIG"
        cp "$WORKSPACE_ROOT/actrix-config.example.toml" "$ACTRIX_CONFIG"
    elif [ -f "$ACTRIX_DIR/config.example.toml" ]; then
        echo "📋 Copying $ACTRIX_DIR/config.example.toml to $ACTRIX_CONFIG"
        cp "$ACTRIX_DIR/config.example.toml" "$ACTRIX_CONFIG"
    else
        echo -e "${RED}❌ No actrix config found. Tried:${NC}"
        echo "   $WORKSPACE_ROOT/actrix-config.toml"
        echo "   $WORKSPACE_ROOT/actrix-config.example.toml"
        echo "   $ACTRIX_DIR/config.example.toml"
        exit 1
    fi
fi

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

# Locate actr CLI
ACTR_GEN_CMD=""

if command -v actr > /dev/null 2>&1; then
    ACTR_GEN_CMD="actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/debug/actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/release/actr"
else
    echo -e "${RED}❌ actr CLI not found (expected 'actr' in PATH or built at $ACTOR_RTC_DIR/actr)${NC}"
    exit 1
fi

if [ ! -d "$PROTO_DIR" ]; then
    echo -e "${RED}❌ Proto directory not found at $PROTO_DIR${NC}"
    exit 1
fi

# Step 0: Generate server code first (no dependencies, can proceed without actrix)
echo ""
echo "🛠️ Generating server code..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$ECHO_SERVER_DIR"
echo "Running actr deps install (server)..."
$ACTR_GEN_CMD deps install || {
    echo -e "${RED}❌ actr deps install failed (server)${NC}"
    exit 1
}
OUTPUT_FILE="$LOG_DIR/actr-gen-echo-server.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed (server)${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Server code generated${NC}"

# Step 1: Build and install actrix
echo ""
echo "📦 Building and installing actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if actrix directory exists
if [ ! -d "$ACTRIX_DIR" ]; then
    echo -e "${RED}❌ Cannot find actrix directory at $ACTRIX_DIR${NC}"
    echo "Please ensure actrix project exists at: $ACTRIX_DIR"
    exit 1
fi

# Build actrix with opentelemetry feature
echo "Building actrix from source (with opentelemetry feature)..."
cd "$ACTRIX_DIR"
cargo build --features opentelemetry 2>&1

# Check if build was successful
if [ ! -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    echo -e "${RED}❌ Failed to build actrix${NC}"
    exit 1
fi

# Copy to ~/.cargo/bin
echo "Installing actrix to ~/.cargo/bin..."
mkdir -p ~/.cargo/bin
cp "$ACTRIX_DIR/target/debug/actrix" ~/.cargo/bin/actrix
chmod +x ~/.cargo/bin/actrix

# Return to workspace root
cd "$WORKSPACE_ROOT"

# Verify actrix is available in PATH
if ! command -v actrix > /dev/null 2>&1; then
    echo -e "${RED}❌ actrix command not found in PATH after installation${NC}"
    echo "Please ensure ~/.cargo/bin is in your PATH"
    exit 1
fi

ACTRIX_CMD="actrix"
echo -e "${GREEN}✅ Actrix built and installed: $(which actrix)${NC}"

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

# Step 2.5a: Setup realms in actrix database directly via sqlite3
echo ""
echo "🔑 Setting up realms in actrix (sqlite3)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait for actrix to initialize its database
sleep 2

# Parse sqlite_path from actrix config (relative paths are relative to WORKSPACE_ROOT)
SQLITE_PATH=$(grep -E '^sqlite_path' "$ACTRIX_CONFIG" | sed 's/.*= *"\(.*\)".*/\1/' | head -1)
if [ -z "$SQLITE_PATH" ]; then
    SQLITE_PATH="database"
fi
# Resolve relative path
case "$SQLITE_PATH" in
    /*) ACTRIX_DB="$SQLITE_PATH/actrix.db" ;;
    *)  ACTRIX_DB="$WORKSPACE_ROOT/$SQLITE_PATH/actrix.db" ;;
esac

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix database not found at $ACTRIX_DB${NC}"
    echo "   Actrix may not have started correctly"
    cat "$LOG_DIR/actrix.log" | tail -20
    exit 1
fi

# Parse realm_id and realm_secret from server's actr.toml
REALM_ID=$(grep -E '^\s*realm_id\s*=' "$ECHO_SERVER_DIR/actr.toml" | sed 's/.*=\s*//' | tr -d ' "' | head -1 || true)
REALM_SECRET=$(grep -E '^\s*realm_secret\s*=' "$ECHO_SERVER_DIR/actr.toml" | sed 's/.*=\s*//' | tr -d ' "' | head -1 || true)

if [ -z "$REALM_ID" ] || [ -z "$REALM_SECRET" ]; then
    echo -e "${RED}❌ Could not parse realm_id or realm_secret from $ECHO_SERVER_DIR/actr.toml${NC}"
    exit 1
fi

echo "   realm_id     = $REALM_ID"
echo "   realm_secret = ${REALM_SECRET:0:10}..."

# Hash the realm secret with SHA256 (matches Rust hash_realm_secret: hex(sha256(secret)))
SECRET_HASH=$(printf '%s' "$REALM_SECRET" | shasum -a 256 | awk '{print $1}')

echo "   secret_hash  = ${SECRET_HASH:0:16}..."

# Insert realm record (OR IGNORE to avoid duplicates)
sqlite3 "$ACTRIX_DB" "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'echo-example', 'Active', 1, strftime('%s','now'), '$SECRET_HASH');"

# Parse ACL rules from actr.toml files and insert into actoracl table
# Server allows: acme:echo-client-app:v1
# Client allows: acme:EchoService:v1
# Note: access=1 means allow
sqlite3 "$ACTRIX_DB" <<EOF
INSERT OR IGNORE INTO actoracl (realm_id, from_type, to_type, access)
VALUES ($REALM_ID, 'acme:EchoService:v1', 'acme:echo-client-app:v1', 1);
INSERT OR IGNORE INTO actoracl (realm_id, from_type, to_type, access)
VALUES ($REALM_ID, 'acme:echo-client-app:v1', 'acme:EchoService:v1', 1);
EOF

echo -e "${GREEN}✅ Realm and ACL setup completed (via sqlite3)${NC}"

# Step 2.5: Build server binary (client code not generated yet)
echo ""
echo "🔨 Building server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin echo-real-server 2>&1; then
    echo -e "${RED}❌ Failed to build server${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Server built successfully${NC}"

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

# Give server a bit more time to fully register for service discovery
sleep 2

# Step 3.5: Install client dependencies and generate code
# Client depends on server (acme:EchoService:v1), so must be done AFTER server registers
echo ""
echo "📎 Installing client dependencies (actr deps install)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$ECHO_CLIENT_DIR"
$ACTR_GEN_CMD deps install || {
    echo -e "${YELLOW}⚠️  actr deps install returned non-zero (client), continuing...${NC}"
}
echo -e "${GREEN}✅ Client dependencies resolved${NC}"

echo ""
echo "🛠️ Generating client code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

OUTPUT_FILE="$LOG_DIR/actr-gen-echo-client.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed (client)${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Client code generated${NC}"

# Step 3.6: Build client binary
echo ""
echo "🔨 Building client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin echo-real-client-app 2>&1; then
    echo -e "${RED}❌ Failed to build client${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Client built successfully${NC}"

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
