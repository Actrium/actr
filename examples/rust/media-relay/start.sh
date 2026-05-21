#!/bin/bash
# Start media-relay example - Using actrix as signaling server
# Auto-starts actrix, actr-b, and actr-a

# Note: not using set -e; we check errors explicitly throughout

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🎬 Media Relay Example - Using Actrix"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine paths and switch to workspace root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTR_DIR="$ACTOR_RTC_DIR/actr"
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
    if [ -f "$ACTRIX_DIR/config.example.toml" ]; then
        ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
        cp "$ACTRIX_DIR/config.example.toml" "$ACTRIX_CONFIG"
    fi
fi
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

# Generate code for actr-b (receiver/server) only
CRATE_DIR="$WORKSPACE_ROOT/media-relay/actr-b"
OUTPUT_FILE="$LOG_DIR/actr-gen-actr-b.log"
echo "🔧 Running actr gen for actr-b (receiver)..."
cd "$CRATE_DIR"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed for actr-b${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ actr gen completed for actr-b (receiver)${NC}"

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

# Step 2.5a: Setup realms in actrix (sqlite3)
echo ""
echo "🔑 Setting up realms in actrix (sqlite3)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

sleep 2

SQLITE_PATH=$(grep -E '^sqlite_path' "$ACTRIX_CONFIG" | sed 's/.*= *"\(.*\)".*/\1/' | head -1 || true)
if [ -z "$SQLITE_PATH" ]; then SQLITE_PATH="database"; fi
case "$SQLITE_PATH" in
    /*) ACTRIX_DB="$SQLITE_PATH/actrix.db" ;;
    *)  ACTRIX_DB="$WORKSPACE_ROOT/$SQLITE_PATH/actrix.db" ;;
esac

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix database not found at $ACTRIX_DB${NC}"
    exit 1
fi

REALM_ID=$(grep -E '^realm_id' "$ACTR_B_DIR/actr.toml" | sed 's/.*= *//' | tr -d ' ' | head -1 || true)
REALM_SECRET=$(grep -E '^realm_secret' "$ACTR_B_DIR/actr.toml" | sed 's/.*= *"\(.*\)".*/\1/' | head -1 || true)
if [ -z "$REALM_ID" ]; then REALM_ID=33554433; fi
if [ -z "$REALM_SECRET" ]; then echo -e "${RED}❌ Could not parse realm_secret${NC}"; exit 1; fi

SECRET_HASH=$(printf '%s' "$REALM_SECRET" | shasum -a 256 | awk '{print $1}')
echo "   realm_id=$REALM_ID secret_hash=${SECRET_HASH:0:16}..."

sqlite3 "$ACTRIX_DB" <<EOF
INSERT OR REPLACE INTO mfr (name, public_key, status, created_at, verified_at) VALUES ('actr-example', '', 'verified', strftime('%s','now'), strftime('%s','now'));
INSERT OR REPLACE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at)
  SELECT id, 'actr-example', 'RelayService', '1.0.0', 'actr-example:RelayService:1.0.0', '{}', '', 'active', strftime('%s','now') FROM mfr WHERE name='actr-example'
  ON CONFLICT(manufacturer, name, version) DO UPDATE SET status='active';
INSERT OR REPLACE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at)
  SELECT id, 'actr-example', 'RelayClient', '1.0.0', 'actr-example:RelayClient:1.0.0', '{}', '', 'active', strftime('%s','now') FROM mfr WHERE name='actr-example'
  ON CONFLICT(manufacturer, name, version) DO UPDATE SET status='active';
INSERT OR REPLACE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'MediaRelay Realm', 'Active', 1, strftime('%s','now'), '$SECRET_HASH');
DELETE FROM actoracl WHERE realm_id = $REALM_ID;
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'actr-example:RelayClient:1.0.0', 'actr-example:RelayService:1.0.0', 1);
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'actr-example:RelayService:1.0.0', 'actr-example:RelayClient:1.0.0', 1);
EOF

echo -e "${GREEN}✅ Realm and ACL setup completed${NC}"

# Step 2.5: Build actr-b (receiver) binary
echo ""
echo "🔨 Building actr-b (receiver) binary..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin actr-b-receiver 2>&1; then
    echo -e "${RED}❌ Failed to build actr-b-receiver${NC}"
    exit 1
fi

echo -e "${GREEN}✅ actr-b-receiver binary built successfully${NC}"

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

    if grep -q "Actr B fully started and registered\|ActrNode started\|ActrNode started" "$LOG_DIR/actr-b.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Actr B is running${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Actr B may still be registering, continuing...${NC}"
fi

sleep 2

# Step 3.5: Install actr-a dependencies (resolve from actrix registry after actr-b registered)
echo ""
echo "📦 Installing actr-a dependencies (actr deps install)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$WORKSPACE_ROOT/media-relay/actr-a"
INSTALL_LOG="$LOG_DIR/actr-install-actr-a.log"
$ACTR_GEN_CMD deps install > "$INSTALL_LOG" 2>&1 || {
    echo -e "${YELLOW}⚠️  actr deps install returned non-zero, check log${NC}"
}
echo -e "${GREEN}✅ actr-a dependencies resolved${NC}"

# Step 3.6: Generate actr-a code
echo ""
echo "🛠️ Generating actr-a code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

OUTPUT_FILE="$LOG_DIR/actr-gen-actr-a.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed for actr-a${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ actr gen completed for actr-a (relay)${NC}"

# Step 3.7: Build actr-a binary
echo ""
echo "🔨 Building actr-a (relay) binary..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin actr-a-relay 2>&1; then
    echo -e "${RED}❌ Failed to build actr-a-relay${NC}"
    exit 1
fi
echo -e "${GREEN}✅ actr-a-relay binary built successfully${NC}"

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

    if grep -q "ActrNode started" "$LOG_DIR/actr-a.log" 2>/dev/null; then
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
tail -15 "$LOG_DIR/actr-b.log" | grep -E "(Received frame|started|success|register)" || tail -15 "$LOG_DIR/actr-b.log"

echo ""
echo "📋 Actr A Output (last 20 lines):"
tail -20 "$LOG_DIR/actr-a.log" | grep -E "(generate frame|sent|complete|started|success)" || tail -20 "$LOG_DIR/actr-a.log"

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
    echo "   • Real ActrNode lifecycle"
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
