#!/bin/bash
# Web Echo Example — Full Package Verification Flow
#
# Demonstrates the complete signing → verification → AIS registration flow for Web:
#   1. Build WASM guests (server + client) to wasm32-unknown-unknown via wasm-pack
#   2. `actr pkg build` — pack WASM + JS glue into signed .actr packages (MFR key)
#   3. Start actrix (signaling + AIS + MFR)
#   4. Seed realm + MFR manufacturer records in DB
#   5. `actr pkg publish` — publish server package to MFR registry
#   6. Copy .actr packages to public/packages/
#   7. Inject MFR public key into actr-config.ts for package verification
#   8. Browser loads .actr → SW verifies Ed25519 sig + SHA-256 hash → AIS register
#
# Usage:
#   ./start.sh

set -e
set -o pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Web Echo (Full Package Verification Flow)"
echo "   sign → verify → AIS register → WASM execute"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Paths ────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# bindings/web is the pnpm workspace root
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"
ACTRIX_DIR="$(cd "$ACTR_ROOT/../actrix" && pwd)"

SERVER_DIR="$SCRIPT_DIR/server"
CLIENT_DIR="$SCRIPT_DIR/client"
SERVER_WASM_DIR="$SERVER_DIR/wasm"
CLIENT_WASM_DIR="$CLIENT_DIR/wasm"
RELEASE_DIR="$SCRIPT_DIR/release"

# MFR manufacturer name (must match actr.toml)
MFR_NAME="acme"
MFR_KEY_FILE=""
MFR_PUBKEY=""

export PATH="$HOME/.cargo/bin:$PATH"

cd "$SCRIPT_DIR"

LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR" "$RELEASE_DIR"

# ── Clean stale data ────────────────────────────────────────────────────

echo ""
echo "🗑️  Cleaning stale data..."
rm -rf "$SCRIPT_DIR/actrix-dev-db"
rm -f "$SCRIPT_DIR/actrix-dev.toml"
rm -f "$SCRIPT_DIR/.actrix.pid" "$SCRIPT_DIR/.server.pid" "$SCRIPT_DIR/.client.pid"

# Restore MFR pubkey placeholder in actr-config.ts files (reset from previous runs)
SERVER_CONFIG="$SERVER_DIR/src/generated/actr-config.ts"
CLIENT_CONFIG="$CLIENT_DIR/src/generated/actr-config.ts"
if [ -f "$SERVER_CONFIG" ]; then
    sed -i '' "s|mfr_pubkey: '[A-Za-z0-9+/=]\{20,\}'|mfr_pubkey: '__MFR_PUBKEY_PLACEHOLDER__'|g" "$SERVER_CONFIG"
fi
if [ -f "$CLIENT_CONFIG" ]; then
    sed -i '' "s|mfr_pubkey: '[A-Za-z0-9+/=]\{20,\}'|mfr_pubkey: '__MFR_PUBKEY_PLACEHOLDER__'|g" "$CLIENT_CONFIG"
fi
echo -e "${GREEN}✅ Stale data cleaned${NC}"

# ── Cleanup ──────────────────────────────────────────────────────────────

ACTRIX_PID=""
SERVER_PID=""
CLIENT_PID=""

cleanup() {
    echo ""
    echo "🧹 Cleaning up..."

    if [ -n "$CLIENT_PID" ]; then
        echo "Stopping web client (PID: $CLIENT_PID)"
        kill $CLIENT_PID 2>/dev/null || true
    fi

    if [ -n "$SERVER_PID" ]; then
        echo "Stopping web server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
    fi

    if [ -n "$ACTRIX_PID" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    # Restore placeholder in actr-config.ts
    if [ -f "$SERVER_CONFIG" ]; then
        sed -i '' "s|mfr_pubkey: '[A-Za-z0-9+/=]\{20,\}'|mfr_pubkey: '__MFR_PUBKEY_PLACEHOLDER__'|g" "$SERVER_CONFIG" 2>/dev/null || true
    fi
    if [ -f "$CLIENT_CONFIG" ]; then
        sed -i '' "s|mfr_pubkey: '[A-Za-z0-9+/=]\{20,\}'|mfr_pubkey: '__MFR_PUBKEY_PLACEHOLDER__'|g" "$CLIENT_CONFIG" 2>/dev/null || true
    fi

    wait 2>/dev/null || true
    echo "✅ Cleanup complete"
}

trap cleanup EXIT INT TERM

# ── Step 0: Check dependencies ──────────────────────────────────────────

echo ""
echo -e "${BLUE}🔍 Step 0: Checking dependencies...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

MISSING=0

if ! command -v node >/dev/null 2>&1; then
    echo -e "${RED}❌ Node.js not found${NC}"
    MISSING=1
else
    echo -e "${GREEN}✅ Node.js: $(node --version)${NC}"
fi

if ! command -v pnpm >/dev/null 2>&1; then
    echo -e "${RED}❌ pnpm not found (install: npm install -g pnpm)${NC}"
    MISSING=1
else
    echo -e "${GREEN}✅ pnpm: $(pnpm --version)${NC}"
fi

if ! command -v wasm-pack >/dev/null 2>&1; then
    echo -e "${YELLOW}⚠️  wasm-pack not found, installing via cargo...${NC}"
    cargo install wasm-pack 2>&1 | tail -3
fi
echo -e "${GREEN}✅ wasm-pack: $(wasm-pack --version 2>&1 | head -1)${NC}"

if [ $MISSING -eq 1 ]; then
    echo -e "${RED}❌ Missing dependencies${NC}"
    exit 1
fi

# ── Step 1: Build WASM guests (wasm-pack) ────────────────────────────────

echo ""
echo -e "${BLUE}📦 Step 1: Building WASM guests via wasm-pack...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

SERVER_WASM_OUT="$RELEASE_DIR/server-wasm"
CLIENT_WASM_OUT="$RELEASE_DIR/client-wasm"
mkdir -p "$SERVER_WASM_OUT" "$CLIENT_WASM_OUT"

# Build server WASM
echo "Building echo-server-web..."
cd "$SERVER_WASM_DIR"
wasm-pack build \
    --target no-modules \
    --out-dir "$SERVER_WASM_OUT" \
    --out-name echo_server \
    --release 2>&1 | tail -5
cd "$SCRIPT_DIR"

SERVER_WASM_FILE="$SERVER_WASM_OUT/echo_server_bg.wasm"
SERVER_JS_FILE="$SERVER_WASM_OUT/echo_server.js"
if [ ! -f "$SERVER_WASM_FILE" ] || [ ! -f "$SERVER_JS_FILE" ]; then
    echo -e "${RED}❌ Server wasm-pack build failed${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Server WASM built: $(du -h "$SERVER_WASM_FILE" | cut -f1)${NC}"

# Build client WASM
echo "Building echo-client-web..."
cd "$CLIENT_WASM_DIR"
wasm-pack build \
    --target no-modules \
    --out-dir "$CLIENT_WASM_OUT" \
    --out-name echo_client \
    --release 2>&1 | tail -5
cd "$SCRIPT_DIR"

CLIENT_WASM_FILE="$CLIENT_WASM_OUT/echo_client_bg.wasm"
CLIENT_JS_FILE="$CLIENT_WASM_OUT/echo_client.js"
if [ ! -f "$CLIENT_WASM_FILE" ] || [ ! -f "$CLIENT_JS_FILE" ]; then
    echo -e "${RED}❌ Client wasm-pack build failed${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Client WASM built: $(du -h "$CLIENT_WASM_FILE" | cut -f1)${NC}"

# ── Step 2: Build signed .actr packages ──────────────────────────────────

echo ""
echo -e "${BLUE}📦 Step 2: Building signed .actr packages...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Ensure actr CLI is available
ACTR_CMD=""
if [ -x "$ACTR_ROOT/target/debug/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/debug/actr"
elif [ -x "$ACTR_ROOT/target/release/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/release/actr"
elif command -v actr > /dev/null 2>&1; then
    ACTR_CMD="actr"
else
    echo -e "${YELLOW}⚠️  actr CLI not found, building...${NC}"
    cd "$ACTR_ROOT"
    cargo build --bin actr 2>&1 | tail -5
    ACTR_CMD="$ACTR_ROOT/target/debug/actr"
    cd "$SCRIPT_DIR"
fi
echo "  Using actr CLI: $ACTR_CMD"

# Generate MFR signing key pair
MFR_KEY_FILE="$RELEASE_DIR/dev-key.json"
$ACTR_CMD pkg keygen --output "$MFR_KEY_FILE" --force
MFR_PUBKEY=$(python3 -c "import json; print(json.load(open('$MFR_KEY_FILE'))['public_key'])")
echo "  MFR pubkey: ${MFR_PUBKEY:0:20}..."

# Build server .actr package
SERVER_ACTR_PACKAGE="$RELEASE_DIR/acme-EchoService-0.1.0-wasm32-unknown-unknown.actr"
$ACTR_CMD pkg build \
    --binary "$SERVER_WASM_FILE" \
    --config "$SERVER_DIR/actr.toml" \
    --key "$MFR_KEY_FILE" \
    --output "$SERVER_ACTR_PACKAGE" \
    --target "wasm32-unknown-unknown" \
    --resource "resources/glue.js=$SERVER_JS_FILE"

if [ ! -f "$SERVER_ACTR_PACKAGE" ]; then
    echo -e "${RED}❌ Server package build failed${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Server .actr: $(du -h "$SERVER_ACTR_PACKAGE" | cut -f1)${NC}"

# Build client .actr package
CLIENT_ACTR_PACKAGE="$RELEASE_DIR/acme-echo-client-app-0.1.0-wasm32-unknown-unknown.actr"
$ACTR_CMD pkg build \
    --binary "$CLIENT_WASM_FILE" \
    --config "$CLIENT_DIR/actr.toml" \
    --key "$MFR_KEY_FILE" \
    --output "$CLIENT_ACTR_PACKAGE" \
    --target "wasm32-unknown-unknown" \
    --resource "resources/glue.js=$CLIENT_JS_FILE"

if [ ! -f "$CLIENT_ACTR_PACKAGE" ]; then
    echo -e "${RED}❌ Client package build failed${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Client .actr: $(du -h "$CLIENT_ACTR_PACKAGE" | cut -f1)${NC}"

# ── Step 3: Start actrix (signaling + AIS + MFR) ────────────────────────

echo ""
echo -e "${BLUE}🚀 Step 3: Starting actrix (signaling + AIS + MFR)...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTRIX_CMD=""
if [ -x "$ACTRIX_DIR/target/release/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
elif [ -x "$ACTRIX_DIR/target/debug/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
elif command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
else
    echo -e "${YELLOW}⚠️  Actrix not found, building...${NC}"
    if [ -d "$ACTRIX_DIR" ]; then
        cd "$ACTRIX_DIR"
        cargo build 2>&1 | tail -5
        ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
        cd "$SCRIPT_DIR"
    fi
    if [ -z "$ACTRIX_CMD" ]; then
        echo -e "${RED}❌ Actrix not available${NC}"
        exit 1
    fi
fi
echo "  Using actrix: $ACTRIX_CMD"

# Create actrix config
cat > "$SCRIPT_DIR/actrix-dev.toml" <<'ACTRIX_EOF'
enable = 25
name = "web-echo-dev"
env = "dev"
sqlite_path = "actrix-dev-db"
location_tag = "local,dev,default"
actrix_shared_key = "web-echo-dev-secret-key-9876543210abcdef"

[recording]
service_name = "web-echo-dev"

[recording.observability]
filter = "digest"

[recording.audit]
filter = "mutations"

[recording.security]
filter = "all"

[recording.operations]
filter = "lifecycle"

[bind.http]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = 8081

[bind.ice]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = 3478
advertised_port = 3478

[turn]
advertised_ip = "127.0.0.1"
advertised_port = 3478
relay_port_range = "49152-49252"
realm = "localhost"

[services.signer]

[services.signer.storage]
backend = "sqlite"
key_ttl_seconds = 3600

[services.signer.storage.sqlite]
path = "actrix-dev-ks.db"

[services.ais]

[services.ais.server]

[services.signaling]

[services.signaling.server]
ws_path = "/signaling"

[control]
head = "admin_ui"

[control.admin_ui]
password = "devpassword123"
session_expiry_secs = 86400

[acl]
enabled = true
default_policy = "allow"
ACTRIX_EOF

$ACTRIX_CMD --config "$SCRIPT_DIR/actrix-dev.toml" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "  Actrix started (PID: $ACTRIX_PID)"
echo "  Waiting for actrix to be ready..."

MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi

    if lsof -i:8081 > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null; then
        echo -e "${GREEN}✅ Actrix is running on port 8081${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix not listening on port 8081 after ${MAX_WAIT}s${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# ── Step 3.5: Seed realm + MFR data ────────────────────────────────────

echo ""
echo -e "${BLUE}🔑 Step 3.5: Seeding realm + MFR data...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

sleep 2

SERVER_REALM=$(grep -E 'realm_id\s*=' "$SERVER_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
CLIENT_REALM=$(grep -E 'realm_id\s*=' "$CLIENT_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')

ACTRIX_DB="$SCRIPT_DIR/actrix-dev-db/actrix.db"

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix DB not found at $ACTRIX_DB${NC}"
    exit 1
fi

NOW=$(date +%s)

for REALM_ID in $SERVER_REALM $CLIENT_REALM; do
    echo "  Creating realm $REALM_ID..."
    sqlite3 "$ACTRIX_DB" \
        "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'web-echo-realm', 'Active', 1, $NOW, '');"
done
echo -e "${GREEN}✅ Realms: $SERVER_REALM, $CLIENT_REALM${NC}"

# Seed MFR manufacturer
EXPIRES_AT=$((NOW + 86400 * 365))
sqlite3 "$ACTRIX_DB" \
    "INSERT OR IGNORE INTO mfr (name, public_key, contact, status, created_at, verified_at, key_expires_at) VALUES ('$MFR_NAME', '$MFR_PUBKEY', 'dev@example.com', 'active', $NOW, $NOW, $EXPIRES_AT);"

MFR_ID=$(sqlite3 "$ACTRIX_DB" "SELECT id FROM mfr WHERE name = '$MFR_NAME';")
echo -e "${GREEN}✅ MFR '$MFR_NAME' registered (id=$MFR_ID)${NC}"

# Seed client package record (client doesn't go through pkg publish)
CLIENT_TYPE_STR="$MFR_NAME:echo-client-app:0.1.0"
sqlite3 "$ACTRIX_DB" \
    "INSERT OR IGNORE INTO mfr_package (mfr_id, manufacturer, name, version, type_str, target, manifest, signature, status, published_at) VALUES ($MFR_ID, '$MFR_NAME', 'echo-client-app', '0.1.0', '$CLIENT_TYPE_STR', 'wasm32-unknown-unknown', '', '', 'active', $NOW);"
echo -e "${GREEN}✅ Client package record seeded${NC}"

# ── Step 4: Publish server .actr package ────────────────────────────────

echo ""
echo -e "${BLUE}📡 Step 4: Publishing server .actr package via 'actr pkg publish'...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

$ACTR_CMD pkg publish \
    --package "$SERVER_ACTR_PACKAGE" \
    --keychain "$MFR_KEY_FILE" \
    --endpoint "http://localhost:8081" \
    --config "$SERVER_DIR/actr.toml"

echo -e "${GREEN}✅ Server package published${NC}"

# ── Step 5: Deploy packages + inject MFR public key ─────────────────────

echo ""
echo -e "${BLUE}📋 Step 5: Deploying .actr packages + injecting MFR pubkey...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Copy .actr packages to public/packages/
mkdir -p "$SERVER_DIR/public/packages" "$CLIENT_DIR/public/packages"
cp "$SERVER_ACTR_PACKAGE" "$SERVER_DIR/public/packages/echo-server.actr"
cp "$CLIENT_ACTR_PACKAGE" "$CLIENT_DIR/public/packages/echo-client.actr"
echo -e "${GREEN}✅ Packages deployed to public/packages/${NC}"

# Sync actor.sw.js from web-sdk source
SW_SRC="$PROJECT_ROOT/packages/web-sdk/src/actor.sw.js"
if [ -f "$SW_SRC" ]; then
    cp "$SW_SRC" "$SERVER_DIR/public/actor.sw.js"
    cp "$SW_SRC" "$CLIENT_DIR/public/actor.sw.js"
    echo -e "${GREEN}✅ actor.sw.js synced from web-sdk${NC}"
else
    echo -e "${YELLOW}⚠️  actor.sw.js not found at $SW_SRC${NC}"
fi

# Inject MFR public key into actr-config.ts (replaces __MFR_PUBKEY_PLACEHOLDER__)
if [ -f "$SERVER_CONFIG" ]; then
    sed -i '' "s|__MFR_PUBKEY_PLACEHOLDER__|${MFR_PUBKEY}|g" "$SERVER_CONFIG"
    echo -e "${GREEN}✅ MFR pubkey injected into server actr-config.ts${NC}"
fi
if [ -f "$CLIENT_CONFIG" ]; then
    sed -i '' "s|__MFR_PUBKEY_PLACEHOLDER__|${MFR_PUBKEY}|g" "$CLIENT_CONFIG"
    echo -e "${GREEN}✅ MFR pubkey injected into client actr-config.ts${NC}"
fi

# ── Step 6: Install web dependencies ────────────────────────────────────

echo ""
echo -e "${BLUE}🌐 Step 6: Installing web dependencies...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$PROJECT_ROOT"
pnpm install 2>&1 | tail -5
echo -e "${GREEN}✅ Dependencies installed${NC}"

# ── Step 7: Start Vite dev servers ──────────────────────────────────────

echo ""
echo -e "${BLUE}🚀 Step 7: Starting Vite dev servers...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Start server (port 5174)
cd "$SERVER_DIR"
pnpm dev > "$LOG_DIR/server.log" 2>&1 &
SERVER_PID=$!
echo "  Server started (PID: $SERVER_PID)"

# Start client (port 5173)
cd "$CLIENT_DIR"
pnpm dev > "$LOG_DIR/client.log" 2>&1 &
CLIENT_PID=$!
echo "  Client started (PID: $CLIENT_PID)"

cd "$SCRIPT_DIR"

# Wait for Vite to start
echo "  Waiting for Vite dev servers..."
sleep 5

if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo -e "${RED}❌ Server failed to start${NC}"
    cat "$LOG_DIR/server.log"
    exit 1
fi
echo -e "${GREEN}✅ Server running at http://localhost:5174${NC}"

if ! kill -0 $CLIENT_PID 2>/dev/null; then
    echo -e "${RED}❌ Client failed to start${NC}"
    cat "$LOG_DIR/client.log"
    exit 1
fi
echo -e "${GREEN}✅ Client running at https://localhost:5173${NC}"

# ── Step 8: Run automated test ──────────────────────────────────────────

echo ""
echo -e "${BLUE}🧪 Step 8: Running automated test...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Resolve puppeteer
if node -e "require('puppeteer')" 2>/dev/null; then
    echo -e "${GREEN}✅ Puppeteer available${NC}"
else
    E2E_MODULES="$PROJECT_ROOT/tests/e2e/node_modules"
    if [ -d "$E2E_MODULES/puppeteer" ]; then
        export NODE_PATH="$E2E_MODULES:${NODE_PATH:-}"
        echo -e "${GREEN}✅ Puppeteer found via workspace tests/e2e${NC}"
    else
        echo -e "${YELLOW}⚠️  Installing puppeteer...${NC}"
        cd "$PROJECT_ROOT"
        PUPPETEER_SKIP_DOWNLOAD=true pnpm add -Dw puppeteer 2>&1 | tail -3
        export NODE_PATH="$PROJECT_ROOT/node_modules:${NODE_PATH:-}"
        cd "$SCRIPT_DIR"
    fi
fi

# Use system Chrome if needed
if ! node -e "require('puppeteer').launch({headless:'new'}).then(b=>b.close())" 2>/dev/null; then
    CHROME_PATH=""
    if [ -f "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" ]; then
        CHROME_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    elif command -v google-chrome >/dev/null 2>&1; then
        CHROME_PATH="$(which google-chrome)"
    elif command -v chromium >/dev/null 2>&1; then
        CHROME_PATH="$(which chromium)"
    fi
    if [ -n "$CHROME_PATH" ]; then
        export PUPPETEER_EXECUTABLE_PATH="$CHROME_PATH"
        echo -e "${GREEN}✅ Using system Chrome: $CHROME_PATH${NC}"
    fi
fi

# Give services time to stabilize
sleep 3

if [ -f "$SCRIPT_DIR/test-auto.js" ]; then
    set +e
    CLIENT_URL="https://localhost:5173" \
    SERVER_URL="http://localhost:5174" \
    node "$SCRIPT_DIR/test-auto.js" BasicFunction
    TEST_EXIT_CODE=$?
    set -e
else
    echo -e "${YELLOW}⚠️  test-auto.js not found, skipping automated test${NC}"
    TEST_EXIT_CODE=-1
fi

# ── Summary ─────────────────────────────────────────────────────────────

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🎉 Web Echo — Full Package Verification Flow"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "✅ Validated flow:"
echo "   1. WASM guests compiled to wasm32-unknown-unknown (wasm-pack)"
echo "   2. actr pkg build → signed .actr packages (MFR key: $MFR_NAME)"
echo "   3. actr pkg publish → server package registered with AIS"
echo "   4. MFR public key injected → SW verifies package signatures"
echo "   5. Browser loads .actr → verifies Ed25519 sig + SHA-256 hash"
echo "   6. SW registers with AIS → obtains credential → starts WebRTC"
echo ""
echo "Services:"
echo "   Actrix:  http://localhost:8081  (signaling + AIS)"
echo "   Server:  http://localhost:5174  (browser-hosted echo service)"
echo "   Client:  https://localhost:5173 (browser echo client)"
echo ""
echo "📖 Logs:"
echo "   tail -f $LOG_DIR/actrix.log"
echo "   tail -f $LOG_DIR/server.log"
echo "   tail -f $LOG_DIR/client.log"
echo ""

if [ $TEST_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}✅ Automated test PASSED${NC}"
elif [ $TEST_EXIT_CODE -eq -1 ]; then
    echo "Press Ctrl+C to stop all services"
    wait
else
    echo -e "${RED}❌ Automated test FAILED (exit code: $TEST_EXIT_CODE)${NC}"
    echo "Services are still running for manual debugging."
    echo "Press Ctrl+C to stop all services"
    wait
fi
