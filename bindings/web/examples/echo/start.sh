#!/bin/bash
# Web Echo Example - Full Package Verification Flow (actr run --web)
#
# Demonstrates the complete signing/verification/AIS registration flow for Web:
#   1. Build guest WASMs (cargo build, standard entry! FFI)
#   2. actr build - pack guest WASMs into signed .actr packages (MFR key)
#   3. Start actrix (signaling + AIS + MFR)
#   4. Seed realm + MFR manufacturer + publish packages (register.sh)
#   5. actr run --web -c server-actr.toml - start server (embedded runtime + host page)
#   6. actr run --web -c client-actr.toml - start client (embedded runtime + host page)
#   7. Run automated test
#
# Usage:
#   ./start.sh

set -e
set -o pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo "Web Echo (actr run --web)"
echo "build -> sign -> register -> actr run --web"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"
ACTRIX_DIR="$(cd "$ACTR_ROOT/../actrix" && pwd)"

SERVER_GUEST_DIR="$SCRIPT_DIR/server-guest"
CLIENT_GUEST_DIR="$SCRIPT_DIR/client-guest"
RELEASE_DIR="$SCRIPT_DIR/release"
SERVER_ACTR_TOML="$SCRIPT_DIR/server-actr.toml"
CLIENT_ACTR_TOML="$SCRIPT_DIR/client-actr.toml"

MFR_NAME="acme"
MFR_KEY_FILE=""
MFR_PUBKEY=""

export PATH="$HOME/.cargo/bin:$PATH"
cd "$SCRIPT_DIR"

LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR" "$RELEASE_DIR"

# ---- Clean stale data ----

echo ""
echo "Cleaning stale data..."
rm -rf "$SCRIPT_DIR/actrix-dev-db"
rm -f "$SCRIPT_DIR/actrix-dev.toml"
rm -f "$SCRIPT_DIR/.actrix.pid" "$SCRIPT_DIR/.server.pid" "$SCRIPT_DIR/.client.pid"

for PORT in 8081 5173 5174; do
    PIDS=$(lsof -ti:$PORT 2>/dev/null || true)
    if [ -n "$PIDS" ]; then
        echo "  Killing existing process(es) on port $PORT: $PIDS"
        echo "$PIDS" | xargs kill -9 2>/dev/null || true
        sleep 0.5
    fi
done

# Reset MFR pubkey placeholder (handles both single and double quotes)
sed -i '' "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" "$SERVER_ACTR_TOML" 2>/dev/null || true
sed -i '' "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" "$CLIENT_ACTR_TOML" 2>/dev/null || true

echo -e "${GREEN}Stale data cleaned${NC}"

# ---- Cleanup handler ----

ACTRIX_PID=""
SERVER_PID=""
CLIENT_PID=""

cleanup() {
    echo ""
    echo "Cleaning up..."
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
    # Restore placeholder (handles both single and double quotes)
    sed -i '' "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" "$SERVER_ACTR_TOML" 2>/dev/null || true
    sed -i '' "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" "$CLIENT_ACTR_TOML" 2>/dev/null || true
    wait 2>/dev/null || true
    echo "Cleanup complete"
}
trap cleanup EXIT INT TERM

# ---- Step 0: Check dependencies ----

echo ""
echo -e "${BLUE}Step 0: Checking dependencies...${NC}"

ACTR_CMD=""
if [ -x "$ACTR_ROOT/target/debug/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/debug/actr"
elif [ -x "$ACTR_ROOT/target/release/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/release/actr"
elif command -v actr > /dev/null 2>&1; then
    ACTR_CMD="actr"
else
    echo -e "${YELLOW}actr CLI not found, building...${NC}"
    cd "$ACTR_ROOT"
    cargo build --bin actr 2>&1 | tail -5
    ACTR_CMD="$ACTR_ROOT/target/debug/actr"
    cd "$SCRIPT_DIR"
fi
echo -e "${GREEN}actr CLI: $ACTR_CMD${NC}"

# ---- Step 1: Build guest WASMs ----

echo ""
echo -e "${BLUE}Step 1: Building guest WASMs...${NC}"

echo "Building echo server guest WASM..."
cd "$SERVER_GUEST_DIR"
cargo build --target wasm32-unknown-unknown --release 2>&1 | tail -5
cd "$SCRIPT_DIR"

SERVER_GUEST_WASM="$SERVER_GUEST_DIR/target/wasm32-unknown-unknown/release/echo_guest.wasm"
if [ ! -f "$SERVER_GUEST_WASM" ]; then
    echo -e "${RED}Server guest WASM build failed${NC}"
    exit 1
fi
echo -e "${GREEN}Server guest WASM: $(du -h "$SERVER_GUEST_WASM" | cut -f1)${NC}"

echo "Building echo-client guest WASM..."
cd "$CLIENT_GUEST_DIR"
cargo build --target wasm32-unknown-unknown --release 2>&1 | tail -5
cd "$SCRIPT_DIR"

CLIENT_GUEST_WASM="$CLIENT_GUEST_DIR/target/wasm32-unknown-unknown/release/echo_client_guest_web.wasm"
if [ ! -f "$CLIENT_GUEST_WASM" ]; then
    echo -e "${RED}Client guest WASM build failed${NC}"
    exit 1
fi
echo -e "${GREEN}Client guest WASM: $(du -h "$CLIENT_GUEST_WASM" | cut -f1)${NC}"

# ---- Step 2: Build signed .actr packages ----

echo ""
echo -e "${BLUE}Step 2: Building signed .actr packages...${NC}"

MFR_KEY_FILE="$RELEASE_DIR/dev-key.json"
$ACTR_CMD pkg keygen --output "$MFR_KEY_FILE" --force
MFR_PUBKEY=$(python3 -c "import json; print(json.load(open('$MFR_KEY_FILE'))['public_key'])")
echo "  MFR pubkey: ${MFR_PUBKEY:0:20}..."

SERVER_ACTR_PACKAGE="$RELEASE_DIR/acme-EchoService-0.1.0-wasm32-unknown-unknown.actr"
$ACTR_CMD pkg build \
    --binary "$SERVER_GUEST_WASM" \
    --config "$SERVER_GUEST_DIR/manifest.toml" \
    --key "$MFR_KEY_FILE" \
    --output "$SERVER_ACTR_PACKAGE" \
    --target "wasm32-unknown-unknown"
if [ ! -f "$SERVER_ACTR_PACKAGE" ]; then
    echo -e "${RED}Server package build failed${NC}"
    exit 1
fi
echo -e "${GREEN}Server .actr: $(du -h "$SERVER_ACTR_PACKAGE" | cut -f1)${NC}"

CLIENT_ACTR_PACKAGE="$RELEASE_DIR/acme-echo-client-app-0.1.0-wasm32-unknown-unknown.actr"
$ACTR_CMD pkg build \
    --binary "$CLIENT_GUEST_WASM" \
    --config "$CLIENT_GUEST_DIR/manifest.toml" \
    --key "$MFR_KEY_FILE" \
    --output "$CLIENT_ACTR_PACKAGE" \
    --target "wasm32-unknown-unknown"
if [ ! -f "$CLIENT_ACTR_PACKAGE" ]; then
    echo -e "${RED}Client package build failed${NC}"
    exit 1
fi
echo -e "${GREEN}Client .actr: $(du -h "$CLIENT_ACTR_PACKAGE" | cut -f1)${NC}"

# ---- Step 3: Start actrix (signaling + AIS + MFR) ----

echo ""
echo -e "${BLUE}Step 3: Starting actrix (signaling + AIS + MFR)...${NC}"

ACTRIX_CMD=""
if [ -x "$ACTRIX_DIR/target/release/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
elif [ -x "$ACTRIX_DIR/target/debug/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
elif command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
else
    echo -e "${YELLOW}Actrix not found, building...${NC}"
    if [ -d "$ACTRIX_DIR" ]; then
        cd "$ACTRIX_DIR"
        cargo build 2>&1 | tail -5
        ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
        cd "$SCRIPT_DIR"
    fi
    if [ -z "$ACTRIX_CMD" ]; then
        echo -e "${RED}Actrix not available${NC}"
        exit 1
    fi
fi
echo "  Using actrix: $ACTRIX_CMD"

cat > "$SCRIPT_DIR/actrix-dev.toml" << 'ACTRIX_EOF'
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
        echo -e "${RED}Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi
    if lsof -i:8081 > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null; then
        echo -e "${GREEN}Actrix is running on port 8081${NC}"
        break
    fi
    sleep 1
    COUNTER=$((COUNTER + 1))
done
if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}Actrix not listening on port 8081 after ${MAX_WAIT}s${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# ---- Step 4: Register realm + MFR + publish packages ----

echo ""
echo -e "${BLUE}Step 4: Registering realm + MFR + publishing packages...${NC}"
sleep 2

bash "$SCRIPT_DIR/register.sh" \
    --db "$SCRIPT_DIR/actrix-dev-db/actrix.db" \
    --endpoint "http://localhost:8081"

echo -e "${GREEN}Registration complete${NC}"

# ---- Step 5: Start actr run --web (server + client) ----

echo ""
echo -e "${BLUE}Step 5: Starting actr run --web (server + client)...${NC}"

# Start server (port 5174)
$ACTR_CMD run --web -c "$SERVER_ACTR_TOML" > "$LOG_DIR/server.log" 2>&1 &
SERVER_PID=$!
echo "  Server started (PID: $SERVER_PID) on port 5174"

# Start client (port 5173)
$ACTR_CMD run --web -c "$CLIENT_ACTR_TOML" > "$LOG_DIR/client.log" 2>&1 &
CLIENT_PID=$!
echo "  Client started (PID: $CLIENT_PID) on port 5173"

echo "  Waiting for web servers..."
sleep 3

if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo -e "${RED}Server failed to start${NC}"
    cat "$LOG_DIR/server.log"
    exit 1
fi
echo -e "${GREEN}Server running at http://localhost:5174${NC}"

if ! kill -0 $CLIENT_PID 2>/dev/null; then
    echo -e "${RED}Client failed to start${NC}"
    cat "$LOG_DIR/client.log"
    exit 1
fi
echo -e "${GREEN}Client running at http://localhost:5173${NC}"

# ---- Step 6: Run automated test ----

echo ""
echo -e "${BLUE}Step 6: Running automated test...${NC}"
sleep 3

TEST_EXIT_CODE=-1
if [ -f "$SCRIPT_DIR/test-auto.js" ]; then
    # Resolve puppeteer
    if ! node -e "require('puppeteer')" 2>/dev/null; then
        E2E_MODULES="$PROJECT_ROOT/tests/e2e/node_modules"
        if [ -d "$E2E_MODULES/puppeteer" ]; then
            export NODE_PATH="$E2E_MODULES:${NODE_PATH:-}"
        fi
    fi

    # Use system Chrome if needed
    if ! node -e "require('puppeteer').launch({headless:'new'}).then(b=>b.close())" 2>/dev/null; then
        CHROME_PATH=""
        if [ -f "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" ]; then
            CHROME_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        elif command -v google-chrome >/dev/null 2>&1; then
            CHROME_PATH="$(which google-chrome)"
        fi
        if [ -n "$CHROME_PATH" ]; then
            export PUPPETEER_EXECUTABLE_PATH="$CHROME_PATH"
        fi
    fi

    set +e
    CLIENT_URL="http://localhost:5173" \
    SERVER_URL="http://localhost:5174" \
    node "$SCRIPT_DIR/test-auto.js" BasicFunction
    TEST_EXIT_CODE=$?
    set -e
else
    echo -e "${YELLOW}test-auto.js not found, skipping automated test${NC}"
fi

# ---- Summary ----

echo ""
echo "Web Echo - actr run --web"
echo ""
echo "Validated flow:"
echo "  1. Guest WASMs built (server-guest + client-guest)"
echo "  2. actr build -> signed .actr packages (MFR key)"
echo "  3. actr pkg publish -> packages registered with AIS"
echo "  4. actr run --web -> self-contained web server with:"
echo "     - Embedded runtime WASM (no wasm-pack step needed)"
echo "     - Embedded actor.sw.js Service Worker"
echo "     - Embedded host page with WebRTC coordinator"
echo "     - .actr packages served from [package].path"
echo "     - Auto-generated /actr-runtime-config.json"
echo ""
echo "Services:"
echo "  Actrix:  http://localhost:8081  (signaling + AIS)"
echo "  Server:  http://localhost:5174  (actr run --web)"
echo "  Client:  http://localhost:5173  (actr run --web)"
echo ""
echo "Logs:"
echo "  tail -f $LOG_DIR/actrix.log"
echo "  tail -f $LOG_DIR/server.log"
echo "  tail -f $LOG_DIR/client.log"
echo ""

if [ $TEST_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}Automated test PASSED${NC}"
elif [ $TEST_EXIT_CODE -eq -1 ]; then
    echo "Press Ctrl+C to stop all services"
    wait
else
    echo -e "${RED}Automated test FAILED (exit code: $TEST_EXIT_CODE)${NC}"
    echo "Services are still running for manual debugging."
    echo "Press Ctrl+C to stop all services"
    wait
fi
