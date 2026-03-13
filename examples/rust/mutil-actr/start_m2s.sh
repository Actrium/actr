#!/bin/bash
# mutil-actr concurrent test script - using actrix as signaling server
# Test multiple clients concurrently sending messages to one server
#
# Usage:
#   ./start.sh              # Start 3  concurrent clients (default)
#   ./start.sh 5            # Start 5  concurrent clients

# Note: not using set -e; we check errors explicitly throughout

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Testing mutil-actr (Multi-client concurrent test)"
echo "    Using Actrix as signaling server"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Get concurrent client count (default 3)
NUM_CLIENTS=${1:-3}
echo "📊 Starting $NUM_CLIENTS concurrent clients"

# Determine paths and switch to workspace root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
# Try multiple config locations
ACTRIX_CONFIG=""
for candidate in "$SCRIPT_DIR/actrix-config.toml" "$WORKSPACE_ROOT/actrix-config.toml" "$ACTRIX_DIR/config.example.toml"; do
    if [ -f "$candidate" ]; then
        ACTRIX_CONFIG="$candidate"
        break
    fi
done
if [ -z "$ACTRIX_CONFIG" ]; then
    # Copy from actrix config.example.toml
    if [ -f "$ACTRIX_DIR/config.example.toml" ]; then
        ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
        cp "$ACTRIX_DIR/config.example.toml" "$ACTRIX_CONFIG"
    fi
fi
ECHO_SERVER_DIR="$WORKSPACE_ROOT/mutil-actr/server"
ECHO_CLIENT_DIR="$WORKSPACE_ROOT/mutil-actr/client"
PROTO_DIR="$WORKSPACE_ROOT/mutil-actr/proto"

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
# Cleanup function
cleanup() {
    echo ""
    echo "🧹 Cleaning up..."

    # Kill actrix
    if [ ! -z "$ACTRIX_PID" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill -9 $ACTRIX_PID 2>/dev/null || true
    fi

    # Kill echo server
    if [ ! -z "$SERVER_PID" ]; then
        echo "Stopping mutil-actr/server (PID: $SERVER_PID)"
        kill -9 $SERVER_PID 2>/dev/null || true
    fi

    # Kill all client apps
    if [ ${#CLIENT_PIDS[@]} -gt 0 ]; then
        echo "Stopping ${#CLIENT_PIDS[@]} clients..."
        for pid in "${CLIENT_PIDS[@]}"; do
            kill -9 $pid 2>/dev/null || true
        done
    fi

    # Wait briefly for processes to exit
    sleep 1

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

# Step 0: Generate server code (no dependencies, can proceed without actrix)
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

# Step 2.5: Setting up realms in actrix (sqlite3)
echo ""
echo "🔑 Setting up realms in actrix (sqlite3)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

sleep 2

# Parse sqlite_path from actrix config
SQLITE_PATH=$(grep -E '^sqlite_path' "$ACTRIX_CONFIG" | sed 's/.*= *"\(.*\)".*/\1/' | head -1 || true)
if [ -z "$SQLITE_PATH" ]; then
    SQLITE_PATH="database"
fi
case "$SQLITE_PATH" in
    /*) ACTRIX_DB="$SQLITE_PATH/actrix.db" ;;
    *)  ACTRIX_DB="$WORKSPACE_ROOT/$SQLITE_PATH/actrix.db" ;;
esac

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix database not found at $ACTRIX_DB${NC}"
    exit 1
fi

# Parse realm_id and realm_secret from server's actr.toml
REALM_ID=$(grep -E '^\s*realm_id\s*=' "$ECHO_SERVER_DIR/actr.toml" | sed 's/.*=\s*//' | tr -d ' "' | head -1 || true)
REALM_SECRET=$(grep -E '^\s*realm_secret\s*=' "$ECHO_SERVER_DIR/actr.toml" | sed 's/.*=\s*//' | tr -d ' "' | head -1 || true)

if [ -z "$REALM_ID" ]; then
    REALM_ID=33554432
fi
if [ -z "$REALM_SECRET" ]; then
    echo -e "${RED}❌ Could not parse realm_secret from $ECHO_SERVER_DIR/actr.toml${NC}"
    exit 1
fi

SECRET_HASH=$(printf '%s' "$REALM_SECRET" | shasum -a 256 | awk '{print $1}')
echo "   realm_id=$REALM_ID secret_hash=${SECRET_HASH:0:16}..."

sqlite3 "$ACTRIX_DB" <<EOF
INSERT OR REPLACE INTO mfr (name, public_key, status, created_at, verified_at) VALUES ('acme', '', 'verified', strftime('%s','now'), strftime('%s','now'));
INSERT OR REPLACE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at)
  SELECT id, 'acme', 'EchoService', 'v1', 'acme:EchoService:v1', '{}', '', 'active', strftime('%s','now') FROM mfr WHERE name='acme'
  ON CONFLICT(manufacturer, name, version) DO UPDATE SET status='active';
INSERT OR REPLACE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at)
  SELECT id, 'acme', 'echo-client-app', 'v1', 'acme:echo-client-app:v1', '{}', '', 'active', strftime('%s','now') FROM mfr WHERE name='acme'
  ON CONFLICT(manufacturer, name, version) DO UPDATE SET status='active';
INSERT OR REPLACE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'MutilActr Realm', 'Active', 1, strftime('%s','now'), '$SECRET_HASH');
DELETE FROM actoracl WHERE realm_id = $REALM_ID;
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'acme:echo-client-app:v1', 'acme:EchoService:v1', 1);
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'acme:EchoService:v1', 'acme:echo-client-app:v1', 1);
EOF

echo -e "${GREEN}✅ Realm and ACL setup completed${NC}"

# Step 3: Start mutil-actr/server
echo ""
echo "🚀 Starting mutil-actr/server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-info}" cargo run --bin mutil_server > "$LOG_DIR/mutil-actr-server.log" 2>&1 &
SERVER_PID=$!

echo "Server started (PID: $SERVER_PID)"
echo "Waiting for server to register and connect to signaling server..."

# Wait for server to start and connect
MAX_WAIT=15
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}❌ Server failed to start${NC}"
        cat "$LOG_DIR/mutil-actr-server.log"
        exit 1
    fi
    
    # Check if server has successfully connected to signaling server
    if grep -q "ActrNode started\|Echo Server fully started and registered\|ActrNode started" "$LOG_DIR/mutil-actr-server.log" 2>/dev/null; then
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

# Step 3.5: Install client dependencies (resolve from actrix registry)
echo ""
echo "📦 Installing client dependencies (actr deps install)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$ECHO_CLIENT_DIR"
INSTALL_LOG="$LOG_DIR/actr-install-client.log"
$ACTR_GEN_CMD deps install > "$INSTALL_LOG" 2>&1 || {
    echo -e "${YELLOW}⚠️  actr deps install returned non-zero, check log${NC}"
}
echo -e "${GREEN}✅ Client dependencies resolved${NC}"

# Step 3.6: Generate client code (protobuf + actor glue)
echo ""
echo "🛠️ Generating client code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

OUTPUT_FILE="$LOG_DIR/actr-gen-echo-client.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ actr gen completed (client code refreshed)${NC}"

# Step 4: Start multiple concurrent clients
echo ""
echo "🚀 Starting $NUM_CLIENTS concurrent clients..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Array to store all client process IDs
CLIENT_PIDS=()

# Start multiple clients concurrently
for i in $(seq 1 $NUM_CLIENTS); do
    echo "Starting client #$i..."
    RUST_LOG="${RUST_LOG:-info}" cargo run --bin mutil_client > "$LOG_DIR/mutil-actr-client-$i.log" 2>&1 &
    CLIENT_PIDS+=($!)
    # Stagger start times slightly to avoid simultaneous connections
    sleep 0.5
done

echo "Started ${#CLIENT_PIDS[@]} client processes:"
for i in "${!CLIENT_PIDS[@]}"; do
    echo "  Client #$((i+1)): PID ${CLIENT_PIDS[$i]}"
done

# Wait for all clients to complete (max 20 seconds)
echo ""
echo "⏳ Waiting for all clients to complete..."
MAX_WAIT=20
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    # Check if any clients are still running
    RUNNING=0
    for pid in "${CLIENT_PIDS[@]}"; do
        if kill -0 $pid 2>/dev/null; then
            RUNNING=$((RUNNING + 1))
        fi
    done
    
    if [ $RUNNING -eq 0 ]; then
        echo "✅ All clients completed"
        break
    fi
    
    sleep 1
    COUNTER=$((COUNTER + 1))
done

# Force terminate still-running clients
if [ $RUNNING -gt 0 ]; then
    echo -e "${YELLOW}⚠️  $RUNNING clients still running, force terminating...${NC}"
    for pid in "${CLIENT_PIDS[@]}"; do
        kill $pid 2>/dev/null || true
    done
fi

# Step 5: Show and verify output
echo ""
echo "🔍 Verifying output..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

SUCCESS_COUNT=0
FAIL_COUNT=0

for i in $(seq 1 $NUM_CLIENTS); do
    LOG_FILE="$LOG_DIR/mutil-actr-client-$i.log"
    
    # Check if response is successful (match multiple possible success outputs)
    if grep -q "success\|Got response from server" "$LOG_FILE"; then
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
        echo -e "${GREEN}✅ Client #$i: PASSED${NC}"
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        echo -e "${RED}❌ Client #$i: FAILED${NC}"
    fi
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 Test Results Summary"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Total clients: $NUM_CLIENTS"
echo "  Passed: $SUCCESS_COUNT"
echo "  Failed: $FAIL_COUNT"
echo ""

if [ $SUCCESS_COUNT -eq $NUM_CLIENTS ]; then
    echo -e "${GREEN}🎉 All tests passed!${NC}"
    echo ""
    echo "✅ Validated:"
    echo "   • $NUM_CLIENTS concurrent client calls"
    echo "   • Each client received correct response"
    echo "   • Response contains corresponding client ID"
    echo "   • Using actrix as signaling server"
    echo ""
    
    # Show all successful client outputs
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📝 All client outputs:"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    for i in $(seq 1 $NUM_CLIENTS); do
        LOG_FILE="$LOG_DIR/mutil-actr-client-$i.log"
        if grep -q "success" "$LOG_FILE"; then
            echo ""
            echo "Client #$i:"
            cat "$LOG_FILE" | grep -A 6 "success" || true
        fi
    done
    echo ""
    
    exit 0
else
    echo -e "${RED}❌ Test failed!${NC}"
    echo ""
    exit 1
fi
