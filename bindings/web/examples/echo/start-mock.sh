#!/usr/bin/env bash
# Web Echo Example — mock-actrix flavored start script.
#
# Same as start.sh but replaces the real actrix binary (+ sqlite3 seeding)
# with the in-repo `actr-mock-actrix` crate. This makes the echo e2e flow
# self-contained: no external actrix project or SQLite schema coupling.
#
# Steps:
#   1. Build guest WASMs (cargo build)
#   2. actr build --no-compile — pack into signed .actr packages
#   3. Start mock-actrix (signaling WS + HTTP AIS + MFR at :8081)
#   4. Seed realm + MFR + packages via HTTP /admin/* (see register-mock.sh)
#   5. actr run --web for server + client
#   6. node test-auto.js BasicFunction
#
# Usage: ./start-mock.sh [MOCK_PORT]

set -e
set -o pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo "Web Echo (mock-actrix)"
echo "build -> sign -> register (mock) -> actr run --web"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"

SERVER_GUEST_DIR="$SCRIPT_DIR/server-guest"
CLIENT_GUEST_DIR="$SCRIPT_DIR/client-guest"
RELEASE_DIR="$SCRIPT_DIR/release"
SERVER_ACTR_TOML="$SCRIPT_DIR/server-actr.toml"
CLIENT_ACTR_TOML="$SCRIPT_DIR/client-actr.toml"

MFR_NAME="acme"
MOCK_PORT="${1:-8081}"

export PATH="$HOME/.cargo/bin:$PATH"
cd "$SCRIPT_DIR"

LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR" "$RELEASE_DIR"

# Portable in-place sed (BSD/macOS + GNU).
sed_inplace() {
    local expr="$1"; shift
    if sed --version >/dev/null 2>&1; then
        # GNU sed
        sed -i "$expr" "$@"
    else
        # BSD sed (macOS)
        sed -i '' "$expr" "$@"
    fi
}

# ---- Clean stale data ----

echo ""
echo "Cleaning stale data..."
rm -f "$SCRIPT_DIR/.mock-actrix.pid" "$SCRIPT_DIR/.server.pid" "$SCRIPT_DIR/.client.pid"

for PORT in "$MOCK_PORT" 5173 5174; do
    PIDS=$(lsof -ti:"$PORT" 2>/dev/null || true)
    if [ -n "$PIDS" ]; then
        echo "  Killing existing process(es) on port $PORT: $PIDS"
        echo "$PIDS" | xargs kill -9 2>/dev/null || true
    fi
done

# Reset MFR pubkey placeholder.
sed_inplace \
    "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
    "$SERVER_ACTR_TOML" 2>/dev/null || true
sed_inplace \
    "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
    "$CLIENT_ACTR_TOML" 2>/dev/null || true

echo -e "${GREEN}Stale data cleaned${NC}"

# ---- Cleanup handler ----

MOCK_PID=""
SERVER_PID=""
CLIENT_PID=""

cleanup() {
    echo ""
    echo "Cleaning up..."
    if [ -n "$CLIENT_PID" ]; then
        kill "$CLIENT_PID" 2>/dev/null || true
    fi
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -n "$MOCK_PID" ]; then
        echo "Stopping mock-actrix (PID: $MOCK_PID)"
        kill "$MOCK_PID" 2>/dev/null || true
    fi
    sed_inplace \
        "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
        "$SERVER_ACTR_TOML" 2>/dev/null || true
    sed_inplace \
        "s|mfr_pubkey = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|mfr_pubkey = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
        "$CLIENT_ACTR_TOML" 2>/dev/null || true
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
    (cd "$ACTR_ROOT" && cargo build --bin actr 2>&1 | tail -5)
    ACTR_CMD="$ACTR_ROOT/target/debug/actr"
fi
echo -e "${GREEN}actr CLI: $ACTR_CMD${NC}"

MOCK_BIN="$ACTR_ROOT/target/debug/mock-actrix"
if [ ! -x "$MOCK_BIN" ]; then
    echo -e "${YELLOW}mock-actrix not built, building...${NC}"
    (cd "$ACTR_ROOT" && cargo build -p actr-mock-actrix --bin mock-actrix 2>&1 | tail -5)
fi
if [ ! -x "$MOCK_BIN" ]; then
    echo -e "${RED}mock-actrix binary not found at $MOCK_BIN${NC}"
    exit 1
fi
echo -e "${GREEN}mock-actrix: $MOCK_BIN${NC}"

# ---- Step 1: Build guest WASMs ----

echo ""
echo -e "${BLUE}Step 1: Building guest WASMs...${NC}"

(cd "$SERVER_GUEST_DIR" && cargo build --target wasm32-unknown-unknown --release 2>&1 | tail -5)
SERVER_GUEST_WASM="$SERVER_GUEST_DIR/target/wasm32-unknown-unknown/release/echo_guest.wasm"
[ -f "$SERVER_GUEST_WASM" ] || { echo -e "${RED}server guest WASM missing${NC}"; exit 1; }

(cd "$CLIENT_GUEST_DIR" && cargo build --target wasm32-unknown-unknown --release 2>&1 | tail -5)
CLIENT_GUEST_WASM="$CLIENT_GUEST_DIR/target/wasm32-unknown-unknown/release/echo_client_guest_web.wasm"
[ -f "$CLIENT_GUEST_WASM" ] || { echo -e "${RED}client guest WASM missing${NC}"; exit 1; }

echo -e "${GREEN}Guest WASMs built${NC}"

# ---- Step 2: Build signed .actr packages ----

echo ""
echo -e "${BLUE}Step 2: Building signed .actr packages...${NC}"

MFR_KEY_FILE="$RELEASE_DIR/dev-key.json"
"$ACTR_CMD" pkg keygen --output "$MFR_KEY_FILE" --force
MFR_PUBKEY=$(python3 -c "import json; print(json.load(open('$MFR_KEY_FILE'))['public_key'])")
echo "  MFR pubkey: ${MFR_PUBKEY:0:20}..."

SERVER_ACTR_PACKAGE="$RELEASE_DIR/acme-EchoService-0.1.0-wasm32-unknown-unknown.actr"
(cd "$SERVER_GUEST_DIR" && "$ACTR_CMD" build \
    --no-compile \
    --target "wasm32-unknown-unknown" \
    --key "$MFR_KEY_FILE" \
    --output "$SERVER_ACTR_PACKAGE")

CLIENT_ACTR_PACKAGE="$RELEASE_DIR/acme-echo-client-app-0.1.0-wasm32-unknown-unknown.actr"
(cd "$CLIENT_GUEST_DIR" && "$ACTR_CMD" build \
    --no-compile \
    --target "wasm32-unknown-unknown" \
    --key "$MFR_KEY_FILE" \
    --output "$CLIENT_ACTR_PACKAGE")

echo -e "${GREEN}.actr packages built${NC}"

# ---- Step 3: Start mock-actrix ----

echo ""
echo -e "${BLUE}Step 3: Starting mock-actrix on port $MOCK_PORT...${NC}"

MOCK_LOG="$LOG_DIR/mock-actrix.log"
: > "$MOCK_LOG"
"$MOCK_BIN" --port "$MOCK_PORT" > "$MOCK_LOG" 2>&1 &
MOCK_PID=$!
echo "  mock-actrix started (PID: $MOCK_PID)"

# Wait for the `listening on` banner (emitted by src/bin/mock_actrix.rs).
READY=0
for _ in $(seq 1 100); do
    if ! kill -0 "$MOCK_PID" 2>/dev/null; then
        echo -e "${RED}mock-actrix exited during startup${NC}"
        cat "$MOCK_LOG"
        exit 1
    fi
    if grep -q "listening on 127.0.0.1:$MOCK_PORT" "$MOCK_LOG"; then
        READY=1
        break
    fi
    # Short wait on the log tail rather than a blind sleep loop.
    sleep 0.1
done
if [ "$READY" -ne 1 ]; then
    echo -e "${RED}mock-actrix did not reach 'listening on' within 10s${NC}"
    cat "$MOCK_LOG"
    exit 1
fi
echo -e "${GREEN}mock-actrix ready on http://127.0.0.1:$MOCK_PORT${NC}"

# ---- Step 4: Seed realm + MFR + packages via HTTP ----

echo ""
echo -e "${BLUE}Step 4: Seeding realm + MFR + packages on mock-actrix...${NC}"

ENDPOINT="http://127.0.0.1:$MOCK_PORT"
bash "$SCRIPT_DIR/register-mock.sh" --endpoint "$ENDPOINT"

echo -e "${GREEN}Registration complete${NC}"

# ---- Step 5: Start actr run --web ----

echo ""
echo -e "${BLUE}Step 5: Starting actr run --web (server + client)...${NC}"

"$ACTR_CMD" run --web -c "$SERVER_ACTR_TOML" > "$LOG_DIR/server.log" 2>&1 &
SERVER_PID=$!
echo "  Server started (PID: $SERVER_PID) on port 5174"

"$ACTR_CMD" run --web -c "$CLIENT_ACTR_TOML" > "$LOG_DIR/client.log" 2>&1 &
CLIENT_PID=$!
echo "  Client started (PID: $CLIENT_PID) on port 5173"

# Wait for both web servers to open their ports.
for PORT in 5173 5174; do
    READY=0
    for _ in $(seq 1 60); do
        if lsof -i:"$PORT" >/dev/null 2>&1 || nc -z 127.0.0.1 "$PORT" 2>/dev/null; then
            READY=1
            break
        fi
        sleep 0.1
    done
    if [ "$READY" -ne 1 ]; then
        echo -e "${RED}port $PORT not bound within 6s${NC}"
        cat "$LOG_DIR/server.log" "$LOG_DIR/client.log" 2>/dev/null || true
        exit 1
    fi
done
echo -e "${GREEN}Server at http://localhost:5174, client at http://localhost:5173${NC}"

# ---- Step 6: Run automated test ----

echo ""
echo -e "${BLUE}Step 6: Running automated test...${NC}"

TEST_EXIT_CODE=-1
if [ -f "$SCRIPT_DIR/test-auto.js" ]; then
    if ! node -e "require('puppeteer')" 2>/dev/null; then
        # Try common locations where puppeteer might be installed.
        for CANDIDATE in \
            "$PROJECT_ROOT/tests/e2e/node_modules" \
            "$PROJECT_ROOT/node_modules" \
            "$ACTR_ROOT/node_modules"; do
            if [ -d "$CANDIDATE" ]; then
                if NODE_PATH="$CANDIDATE" node -e "require('puppeteer')" 2>/dev/null; then
                    export NODE_PATH="$CANDIDATE:${NODE_PATH:-}"
                    break
                fi
            fi
        done
    fi

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
echo "Services:"
echo "  mock-actrix: http://127.0.0.1:$MOCK_PORT (ws://127.0.0.1:$MOCK_PORT/signaling/ws)"
echo "  Server:      http://localhost:5174"
echo "  Client:      http://localhost:5173"
echo ""
echo "Logs:"
echo "  tail -f $LOG_DIR/mock-actrix.log"
echo "  tail -f $LOG_DIR/server.log"
echo "  tail -f $LOG_DIR/client.log"

if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo -e "${GREEN}Automated test PASSED${NC}"
elif [ "$TEST_EXIT_CODE" -eq -1 ]; then
    echo "Press Ctrl+C to stop all services"
    wait
else
    echo -e "${RED}Automated test FAILED (exit code: $TEST_EXIT_CODE)${NC}"
    echo "Services still running for manual debugging. Press Ctrl+C to stop."
    wait
fi
