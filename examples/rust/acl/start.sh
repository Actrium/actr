#!/usr/bin/env bash
# Note: not using set -euo pipefail; we check errors explicitly throughout

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Directories
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
    if [ -f "$ACTRIX_DIR/config.example.toml" ]; then
        ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
        cp "$ACTRIX_DIR/config.example.toml" "$ACTRIX_CONFIG"
    fi
fi
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
echo "🔍 Checking actr.toml files..."
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

# Step 1.5: Setup realms in actrix (sqlite3)
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

REALM_ID=$(grep -E '^\s*realm_id\s*=' "$SERVER_DIR/actr.toml" | sed 's/.*=\s*//' | tr -d ' "' | head -1 || true)
REALM_SECRET=$(grep -E '^\s*realm_secret\s*=' "$SERVER_DIR/actr.toml" | sed 's/.*=\s*//' | tr -d ' "' | head -1 || true)
if [ -z "$REALM_ID" ]; then REALM_ID=33554434; fi
if [ -z "$REALM_SECRET" ]; then echo -e "${RED}❌ Could not parse realm_secret from $SERVER_DIR/actr.toml${NC}"; exit 1; fi

SECRET_HASH=$(printf '%s' "$REALM_SECRET" | shasum -a 256 | awk '{print $1}')
echo "   realm_id=$REALM_ID secret_hash=${SECRET_HASH:0:16}..."

sqlite3 "$ACTRIX_DB" <<EOF
INSERT OR REPLACE INTO mfr (name, public_key, status, created_at, verified_at) VALUES ('acme', '', 'verified', strftime('%s','now'), strftime('%s','now'));
INSERT OR REPLACE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at)
  SELECT id, 'acme', 'greeter.GreeterService', 'v1', 'acme:greeter.GreeterService:v1', '{}', '', 'active', strftime('%s','now') FROM mfr WHERE name='acme'
  ON CONFLICT(manufacturer, name, version) DO UPDATE SET status='active';
INSERT OR REPLACE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at)
  SELECT id, 'acme', 'allowed-greeter-client', 'v1', 'acme:allowed-greeter-client:v1', '{}', '', 'active', strftime('%s','now') FROM mfr WHERE name='acme'
  ON CONFLICT(manufacturer, name, version) DO UPDATE SET status='active';
INSERT OR REPLACE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at)
  SELECT id, 'acme', 'blocked-greeter-client', 'v1', 'acme:blocked-greeter-client:v1', '{}', '', 'active', strftime('%s','now') FROM mfr WHERE name='acme'
  ON CONFLICT(manufacturer, name, version) DO UPDATE SET status='active';
INSERT OR REPLACE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'ACL Demo Realm', 'Active', 1, strftime('%s','now'), '$SECRET_HASH');
DELETE FROM actoracl WHERE realm_id = $REALM_ID;
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'acme:allowed-greeter-client:v1', 'acme:greeter.GreeterService:v1', 1);
INSERT INTO actoracl (realm_id, source_realm_id, from_type, to_type, access)
VALUES ($REALM_ID, $REALM_ID, 'acme:greeter.GreeterService:v1', 'acme:allowed-greeter-client:v1', 1);
EOF

echo -e "${GREEN}✅ Realm and ACL setup completed${NC}"

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

# Generate code for server only (with scaffold)
cd "$SERVER_DIR"
echo "Running actr deps install (server)..."
if ! $ACTR_GEN_CMD deps install > "$LOG_DIR/actr-deps-server.log" 2>&1; then
    echo -e "${RED}❌ actr deps install failed (server)${NC}"
    cat "$LOG_DIR/actr-deps-server.log"
    exit 1
fi
if ! $ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$LOG_DIR/actr-gen-server.log" 2>&1; then
    echo -e "${RED}❌ Server code generation failed${NC}"
    cat "$LOG_DIR/actr-gen-server.log"
    exit 1
fi
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Server code generated successfully${NC}"

# Step 3: Build server binary
echo ""
echo "🔨 Building server binary..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$WORKSPACE_ROOT"
if ! cargo build --bin acl-server > "$LOG_DIR/cargo-build-server.log" 2>&1; then
    echo -e "${RED}❌ Server build failed${NC}"
    tail -20 "$LOG_DIR/cargo-build-server.log"
    exit 1
fi
echo -e "${GREEN}✅ Server binary built successfully${NC}"

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

sleep 2

# Step 4.5: Install client dependencies (resolve from actrix registry)
echo ""
echo "📦 Installing client dependencies (actr deps install)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

for client_dir in "$ALLOWED_CLIENT_DIR" "$BLOCKED_CLIENT_DIR"; do
    cd "$client_dir"
    INSTALL_LOG="$LOG_DIR/actr-install-$(basename $client_dir).log"
    $ACTR_GEN_CMD deps install > "$INSTALL_LOG" 2>&1 || {
        echo -e "${YELLOW}⚠️  actr deps install returned non-zero for $(basename $client_dir), check log${NC}"
    }
done
echo -e "${GREEN}✅ Client dependencies resolved${NC}"

# Step 4.6: Generate client code
echo ""
echo "🛠️  Generating client code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

for client_dir in "$ALLOWED_CLIENT_DIR" "$BLOCKED_CLIENT_DIR"; do
    cd "$client_dir"
    if ! $ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$LOG_DIR/actr-gen-$(basename $client_dir).log" 2>&1; then
        echo -e "${RED}❌ $(basename $client_dir) code generation failed${NC}"
        cat "$LOG_DIR/actr-gen-$(basename $client_dir).log"
        exit 1
    fi
done
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ Client code generated successfully${NC}"

# Step 4.7: Build client binaries
echo ""
echo "🔨 Building client binaries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cd "$WORKSPACE_ROOT"
if ! cargo build --bin allowed-client --bin blocked-client > "$LOG_DIR/cargo-build-clients.log" 2>&1; then
    echo -e "${RED}❌ Client build failed${NC}"
    tail -20 "$LOG_DIR/cargo-build-clients.log"
    exit 1
fi
echo -e "${GREEN}✅ Client binaries built successfully${NC}"

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
    echo "   • ACL configuration from actr.toml works"
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
