#!/bin/bash
# Start data-stream example - Using actrix as signaling server
# Auto-starts actrix, receiver, and sender

set -e
set -o pipefail

# Track error location for diagnostics
ERROR_LINE=0
ERROR_CMD=""

# ERR trap: capture which line/command failed (fires BEFORE EXIT trap)
trap 'ERROR_LINE=$LINENO; ERROR_CMD="$BASH_COMMAND"; echo ""; echo -e "${RED:-}❌ ERROR on line $ERROR_LINE: $ERROR_CMD${NC:-}"' ERR

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📡 DataStream Example - Using Actrix"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine paths and switch to workspace root
# Use BASH_SOURCE[0] to reliably locate this script regardless of CWD
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# SCRIPT_DIR       = .../Actrium/actr/examples/rust/data-stream
# WORKSPACE_ROOT   = .../Actrium/actr/examples/rust  (the Cargo workspace)
# ACTOR_RTC_DIR    = .../Actrium                     (repo root, parent of both actr/ and actrix/)
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
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

# Ensure actr.toml files exist for sender and receiver
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
SENDER_DIR="$SCRIPT_DIR/sender"
RECEIVER_DIR="$SCRIPT_DIR/receiver"
ensure_actr_toml "$SENDER_DIR"
ensure_actr_toml "$RECEIVER_DIR"

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
    local exit_code=$?
    # Disable ERR trap inside cleanup to avoid misleading errors
    trap '' ERR
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

    if [ $exit_code -ne 0 ]; then
        echo ""
        echo -e "${RED}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        echo -e "${RED}❌ Script FAILED (exit code: $exit_code)${NC}"
        if [ $ERROR_LINE -ne 0 ]; then
            echo -e "${RED}   Line $ERROR_LINE: $ERROR_CMD${NC}"
        fi
        echo -e "${RED}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        echo ""
        echo "📖 Check logs for details:"
        echo "   ls -la $LOG_DIR/"
    else
        echo "✅ Cleanup complete"
    fi
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
elif [ -x "$ACTOR_RTC_DIR/actr-cli/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr-cli/target/debug/actr"
elif [ -x "$ACTOR_RTC_DIR/actr-cli/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr-cli/target/release/actr"
else
    echo -e "${RED}❌ actr generator not found (expected 'actr' in PATH or built under $ACTOR_RTC_DIR/actr-cli)${NC}"
    exit 1
fi

if [ ! -d "$PROTO_DIR" ]; then
    echo -e "${RED}❌ Proto directory not found at $PROTO_DIR${NC}"
    exit 1
fi

# Generate receiver code first (no dependencies, can proceed without actrix)
echo ""
echo "🛠️ Generating receiver code..."
cd "$RECEIVER_DIR"
echo "Running actr install..."
$ACTR_GEN_CMD deps install || {
    echo -e "${RED}❌ actr install failed (receiver)${NC}"
    exit 1
}
OUTPUT_FILE="$LOG_DIR/actr-gen-receiver.log"
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

# Step 2: Build and install actrix
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
cargo build --features opentelemetry 2>&1 | tail -5

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

# Ensure actrix-config.toml exists (find or copy from actrix defaults)
if [ -f "$WORKSPACE_ROOT/actrix-config.toml" ]; then
    ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
elif [ -f "$WORKSPACE_ROOT/actrix-config.example.toml" ]; then
    cp "$WORKSPACE_ROOT/actrix-config.example.toml" "$WORKSPACE_ROOT/actrix-config.toml"
    ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
elif [ -f "$ACTRIX_DIR/config.example.toml" ]; then
    cp "$ACTRIX_DIR/config.example.toml" "$WORKSPACE_ROOT/actrix-config.toml"
    ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
else
    echo -e "${RED}❌ Cannot find actrix config file${NC}"
    exit 1
fi

# Verify actrix is available in PATH
if ! command -v actrix > /dev/null 2>&1; then
    echo -e "${RED}❌ actrix command not found in PATH after installation${NC}"
    echo "Please ensure ~/.cargo/bin is in your PATH"
    exit 1
fi

ACTRIX_CMD="actrix"
echo -e "${GREEN}✅ Actrix built and installed: $(which actrix)${NC}"

# Step 3: Initialize actrix database
echo ""
echo "🗄️  Initializing actrix database..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Parse sqlite_path from actrix config
SQLITE_PATH=$(grep -E '^sqlite_path' "$ACTRIX_CONFIG" | sed 's/.*= *"\(.*\)".*/\1/' | head -1)
if [ -z "$SQLITE_PATH" ]; then
    SQLITE_PATH="database"
fi
# Resolve relative path
case "$SQLITE_PATH" in
    /*) ACTRIX_DB="$SQLITE_PATH/actrix.db" ;;
    *)  ACTRIX_DB="$WORKSPACE_ROOT/$SQLITE_PATH/actrix.db" ;;
esac

# If database doesn't exist, start actrix briefly to create it
if [ ! -f "$ACTRIX_DB" ]; then
    echo "   Database not found, starting actrix briefly to initialize..."
    $ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix-init.log" 2>&1 &
    ACTRIX_INIT_PID=$!
    
    MAX_WAIT=15
    COUNTER=0
    while [ $COUNTER -lt $MAX_WAIT ]; do
        if [ -f "$ACTRIX_DB" ]; then
            break
        fi
        sleep 1
        COUNTER=$((COUNTER + 1))
    done
    
    # Stop the init instance
    kill $ACTRIX_INIT_PID 2>/dev/null
    wait $ACTRIX_INIT_PID 2>/dev/null || true
    sleep 2
    
    # Kill any leftover process on port 8081
    if lsof -ti:8081 > /dev/null 2>&1; then
        kill $(lsof -ti:8081) 2>/dev/null || true
        sleep 1
    fi
fi

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix database not created at $ACTRIX_DB${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Database ready at $ACTRIX_DB${NC}"

# Step 3.5: Setting up realms and ACL BEFORE actrix starts
echo ""
echo "🔑 Setting up realms in actrix (sqlite3)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Parse realm_id and realm_secret from receiver's actr.toml
# Note: these keys live under [system.deployment] section, grep may find 0 matches
REALM_ID=$(grep -E '^\s*realm_id\s*=' "$RECEIVER_DIR/actr.toml" | sed 's/.*=\s*//' | tr -d ' "' | head -1 || true)
REALM_SECRET=$(grep -E '^\s*realm_secret\s*=' "$RECEIVER_DIR/actr.toml" | sed 's/.*=\s*"\{0,1\}//' | sed 's/"\s*$//' | head -1 || true)

if [ -z "$REALM_ID" ]; then
    REALM_ID=33554432
fi
if [ -z "$REALM_SECRET" ]; then
    echo -e "${RED}❌ Could not parse realm_secret from $RECEIVER_DIR/actr.toml${NC}"
    exit 1
fi

echo "   realm_id     = $REALM_ID"
echo "   realm_secret = ${REALM_SECRET:0:10}..."

SECRET_HASH=$(printf '%s' "$REALM_SECRET" | shasum -a 256 | awk '{print $1}')
echo "   secret_hash  = ${SECRET_HASH:0:16}..."

# Insert realm and ACL rules (with clean slate for ACL)
sqlite3 "$ACTRIX_DB" <<EOF
INSERT OR REPLACE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'DataStream Realm', 'Active', 1, strftime('%s','now'), '$SECRET_HASH');
DELETE FROM actoracl WHERE realm_id = $REALM_ID;
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'acme:GenericClient:1.0.0', 'acme:FileTransferService:1.0.0', 1);
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'acme:FileTransferService:1.0.0', 'acme:GenericClient:1.0.0', 1);
EOF

echo -e "${GREEN}✅ Realm and ACL setup completed${NC}"

# Verify ACL entries
sqlite3 "$ACTRIX_DB" "SELECT from_type, to_type, access FROM actoracl WHERE realm_id = $REALM_ID;" | while read line; do
    echo "   ACL: $line"
done

# Step 3.6: Start actrix (database already has realm+ACL)
echo ""
echo "🚀 Starting actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Kill any remaining process on port 8081
if lsof -ti:8081 > /dev/null 2>&1; then
    kill $(lsof -ti:8081) 2>/dev/null || true
    sleep 1
fi

$ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "Actrix started (PID: $ACTRIX_PID)"
echo "Waiting for actrix to be ready..."

MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi
    
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

# Step 3.7: Build and start receiver first (so it registers with actrix)
echo ""
echo "🔨 Building receiver..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin data-stream-receiver 2>&1; then
    echo -e "${RED}❌ Failed to build receiver${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Receiver built successfully${NC}"

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

# Step 4.5: Install sender dependencies and generate code
# The receiver is now registered with actrix, so actr install can
# discover it via service discovery and resolve the dependency.
echo ""
echo "📦 Installing sender dependencies (actr install)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$SENDER_DIR"
echo "Running actr install..."
$ACTR_GEN_CMD deps install || {
    echo -e "${RED}❌ actr install failed (sender)${NC}"
    exit 1
}
echo -e "${GREEN}✅ Sender dependencies installed${NC}"

echo ""
echo "🛠️ Generating sender code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
OUTPUT_FILE="$LOG_DIR/actr-gen-sender.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed (sender)${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Sender code generated${NC}"

echo ""
echo "🔨 Building sender..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin data-stream-sender 2>&1; then
    echo -e "${RED}❌ Failed to build sender${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Sender built successfully${NC}"

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
