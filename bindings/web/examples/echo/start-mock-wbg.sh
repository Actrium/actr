#!/usr/bin/env bash
# Web Echo Example — Option U (wasm-bindgen guest) flavored start script.
#
# Mirrors `start-mock.sh` but:
#   - builds the `*-guest-wbg` variants via wasm-pack (no Component Model,
#     no jco) instead of the wasip2 Component Model variants
#   - lays out the wasm-pack output under the `<stem>.wbg/` sibling of the
#     signed .actr, matching the convention `cli/src/commands/run.rs`
#     expects (see `wbg_dir` resolution)
#   - exports `ACTR_WEB_GUEST_MODE=wbg` so `actr run --web` serves
#     `actor-wbg.sw.js` instead of the default `actor.sw.js`
#
# The Component-Model `start-mock.sh` path is untouched so other examples
# (data-stream-peer-concurrent, etc.) keep working.
#
# Usage: ./start-mock-wbg.sh [MOCK_PORT]

set -e
set -o pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo "Web Echo (mock-actrix, Option U / wasm-bindgen guest)"
echo "build (wasm-pack) -> sign -> register (mock) -> actr run --web (WBG mode)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"

# The wasip2 Component guest crates (unchanged): `actr build --no-compile`
# still needs *some* core wasm inside the .actr to pass verification. We
# reuse the Component-Model produced binary so the .actr stays signable the
# same way; the WBG path only uses the .actr for verify + metadata, the
# actual code shipped to the SW is the wasm-pack output.
SERVER_GUEST_CM_DIR="$SCRIPT_DIR/server-guest"
CLIENT_GUEST_CM_DIR="$SCRIPT_DIR/client-guest"

# The new WBG variants — built as plain wasm32-unknown-unknown via
# wasm-pack. These produce `pkg/*_bg.wasm` + `pkg/*.js` glue.
SERVER_GUEST_WBG_DIR="$SCRIPT_DIR/server-guest-wbg"
CLIENT_GUEST_WBG_DIR="$SCRIPT_DIR/client-guest-wbg"

RELEASE_DIR="$SCRIPT_DIR/release"
SERVER_ACTR_TOML="$SCRIPT_DIR/server-actr.toml"
CLIENT_ACTR_TOML="$SCRIPT_DIR/client-actr.toml"

MFR_NAME="acme"
MOCK_PORT="${1:-8081}"

export PATH="$HOME/.cargo/bin:$PATH"
cd "$SCRIPT_DIR"

LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR" "$RELEASE_DIR"

sed_inplace() {
    local expr="$1"; shift
    if sed --version >/dev/null 2>&1; then
        sed -i "$expr" "$@"
    else
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

sed_inplace \
    "s|pubkey_b64 = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|pubkey_b64 = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
    "$SERVER_ACTR_TOML" 2>/dev/null || true
sed_inplace \
    "s|pubkey_b64 = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|pubkey_b64 = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
    "$CLIENT_ACTR_TOML" 2>/dev/null || true

echo -e "${GREEN}Stale data cleaned${NC}"

MOCK_PID=""
SERVER_PID=""
CLIENT_PID=""

cleanup() {
    echo ""
    echo "Cleaning up..."
    if [ -n "$CLIENT_PID" ]; then kill "$CLIENT_PID" 2>/dev/null || true; fi
    if [ -n "$SERVER_PID" ]; then kill "$SERVER_PID" 2>/dev/null || true; fi
    if [ -n "$MOCK_PID" ]; then
        echo "Stopping mock-actrix (PID: $MOCK_PID)"
        kill "$MOCK_PID" 2>/dev/null || true
    fi
    sed_inplace \
        "s|pubkey_b64 = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|pubkey_b64 = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
        "$SERVER_ACTR_TOML" 2>/dev/null || true
    sed_inplace \
        "s|pubkey_b64 = [\"'][A-Za-z0-9+/=]\{20,\}[\"']|pubkey_b64 = \"__MFR_PUBKEY_PLACEHOLDER__\"|g" \
        "$CLIENT_ACTR_TOML" 2>/dev/null || true
    wait 2>/dev/null || true
    echo "Cleanup complete"
}
trap cleanup EXIT INT TERM

# ---- Step 0: dependencies ----

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
[ -x "$MOCK_BIN" ] || { echo -e "${RED}mock-actrix not found at $MOCK_BIN${NC}"; exit 1; }
echo -e "${GREEN}mock-actrix: $MOCK_BIN${NC}"

if ! command -v wasm-pack >/dev/null 2>&1; then
    echo -e "${RED}wasm-pack not found (install: cargo install wasm-pack)${NC}"
    exit 1
fi

# Component Model toolchain — still needed to build the signed .actr whose
# verification the WBG SW path still runs. Kept identical to start-mock.sh
# so a broken wasm-component-ld surfaces the same way.
if [ -x "${WASM_COMPONENT_LD:-$HOME/.cargo/bin/wasm-component-ld}" ]; then
    WASM_COMPONENT_LD="${WASM_COMPONENT_LD:-$HOME/.cargo/bin/wasm-component-ld}"
elif command -v wasm-component-ld > /dev/null 2>&1; then
    WASM_COMPONENT_LD="$(command -v wasm-component-ld)"
else
    echo -e "${RED}wasm-component-ld not found${NC}"; exit 1
fi

# ---- Step 1a: build CM guests (for .actr signing path only) ----

echo ""
echo -e "${BLUE}Step 1a: Building CM guests for .actr signing...${NC}"

(
    export RUSTFLAGS="-Clinker=$WASM_COMPONENT_LD"
    cd "$SERVER_GUEST_CM_DIR" && cargo build --target wasm32-wasip2 --release 2>&1 | tail -5
)
SERVER_GUEST_CM_WASM="$SERVER_GUEST_CM_DIR/target/wasm32-wasip2/release/echo_guest.wasm"
[ -f "$SERVER_GUEST_CM_WASM" ] || { echo -e "${RED}server CM guest missing${NC}"; exit 1; }

(
    export RUSTFLAGS="-Clinker=$WASM_COMPONENT_LD"
    cd "$CLIENT_GUEST_CM_DIR" && cargo build --target wasm32-wasip2 --release 2>&1 | tail -5
)
CLIENT_GUEST_CM_WASM="$CLIENT_GUEST_CM_DIR/target/wasm32-wasip2/release/echo_client_guest_web.wasm"
[ -f "$CLIENT_GUEST_CM_WASM" ] || { echo -e "${RED}client CM guest missing${NC}"; exit 1; }

echo -e "${GREEN}CM guests built${NC}"

# ---- Step 1b: build WBG guests via wasm-pack ----

echo ""
echo -e "${BLUE}Step 1b: Building WBG guests via wasm-pack...${NC}"

# `~/.cargo/config.toml` injects `-fuse-ld=mold` for host builds; rust-lld
# (the wasm32 linker) rejects that flag. Clear the rustflags explicitly
# only for the wasm-pack invocations — other steps still need them.
(
    export RUSTFLAGS=""
    export CARGO_ENCODED_RUSTFLAGS=""
    cd "$SERVER_GUEST_WBG_DIR"
    wasm-pack build --target no-modules --release --out-dir pkg 2>&1 | tail -8
)
SERVER_WBG_JS="$SERVER_GUEST_WBG_DIR/pkg/echo_server_guest_wbg.js"
SERVER_WBG_WASM="$SERVER_GUEST_WBG_DIR/pkg/echo_server_guest_wbg_bg.wasm"
[ -f "$SERVER_WBG_JS" ] && [ -f "$SERVER_WBG_WASM" ] || { echo -e "${RED}server WBG pkg incomplete${NC}"; exit 1; }

(
    export RUSTFLAGS=""
    export CARGO_ENCODED_RUSTFLAGS=""
    cd "$CLIENT_GUEST_WBG_DIR"
    wasm-pack build --target no-modules --release --out-dir pkg 2>&1 | tail -8
)
CLIENT_WBG_JS="$CLIENT_GUEST_WBG_DIR/pkg/echo_client_guest_wbg.js"
CLIENT_WBG_WASM="$CLIENT_GUEST_WBG_DIR/pkg/echo_client_guest_wbg_bg.wasm"
[ -f "$CLIENT_WBG_JS" ] && [ -f "$CLIENT_WBG_WASM" ] || { echo -e "${RED}client WBG pkg incomplete${NC}"; exit 1; }

echo -e "${GREEN}WBG guests built${NC}"

# ---- Step 2: sign .actr packages (Component binary inside) ----

echo ""
echo -e "${BLUE}Step 2: Building signed .actr packages...${NC}"

MFR_KEY_FILE="$RELEASE_DIR/dev-key.json"
"$ACTR_CMD" pkg keygen --output "$MFR_KEY_FILE" --force
MFR_PUBKEY=$(python3 -c "import json; print(json.load(open('$MFR_KEY_FILE'))['public_key'])")
echo "  MFR pubkey: ${MFR_PUBKEY:0:20}..."

SERVER_ACTR_PACKAGE="$RELEASE_DIR/acme-EchoService-0.1.0-wasm32-wasip2.actr"
(cd "$SERVER_GUEST_CM_DIR" && "$ACTR_CMD" build \
    --no-compile \
    --target "wasm32-wasip2" \
    --key "$MFR_KEY_FILE" \
    --output "$SERVER_ACTR_PACKAGE")

CLIENT_ACTR_PACKAGE="$RELEASE_DIR/acme-echo-client-app-0.1.0-wasm32-wasip2.actr"
(cd "$CLIENT_GUEST_CM_DIR" && "$ACTR_CMD" build \
    --no-compile \
    --target "wasm32-wasip2" \
    --key "$MFR_KEY_FILE" \
    --output "$CLIENT_ACTR_PACKAGE")

echo -e "${GREEN}.actr packages built${NC}"

# ---- Step 2b: lay out `<stem>.wbg/` sibling bundles ----
#
# The WBG SW entry resolves `<packageUrl>.wbg/guest.js` by default. CLI
# mounts `<package stem>.wbg/` under `/packages/<stem>.wbg/` when it
# exists. We rename the wasm-pack output to the conventional `guest.js` /
# `guest_bg.wasm` pair so the SW URL resolution is trivial.

SERVER_WBG_DIR="$RELEASE_DIR/acme-EchoService-0.1.0-wasm32-wasip2.wbg"
CLIENT_WBG_DIR="$RELEASE_DIR/acme-echo-client-app-0.1.0-wasm32-wasip2.wbg"
rm -rf "$SERVER_WBG_DIR" "$CLIENT_WBG_DIR"
mkdir -p "$SERVER_WBG_DIR" "$CLIENT_WBG_DIR"

cp "$SERVER_WBG_JS"   "$SERVER_WBG_DIR/guest.js"
cp "$SERVER_WBG_WASM" "$SERVER_WBG_DIR/guest_bg.wasm"
cp "$CLIENT_WBG_JS"   "$CLIENT_WBG_DIR/guest.js"
cp "$CLIENT_WBG_WASM" "$CLIENT_WBG_DIR/guest_bg.wasm"

echo -e "${GREEN}WBG sibling bundles laid out under release/*.wbg/${NC}"

# ---- Step 3: start mock-actrix ----

echo ""
echo -e "${BLUE}Step 3: Starting mock-actrix on port $MOCK_PORT...${NC}"

MOCK_LOG="$LOG_DIR/mock-actrix.log"
: > "$MOCK_LOG"
"$MOCK_BIN" --port "$MOCK_PORT" > "$MOCK_LOG" 2>&1 &
MOCK_PID=$!
echo "  mock-actrix started (PID: $MOCK_PID)"

READY=0
for _ in $(seq 1 100); do
    if ! kill -0 "$MOCK_PID" 2>/dev/null; then
        echo -e "${RED}mock-actrix exited during startup${NC}"
        cat "$MOCK_LOG"; exit 1
    fi
    if grep -q "listening on 127.0.0.1:$MOCK_PORT" "$MOCK_LOG"; then
        READY=1; break
    fi
    sleep 0.1
done
[ "$READY" -eq 1 ] || { echo -e "${RED}mock-actrix did not reach 'listening on' within 10s${NC}"; cat "$MOCK_LOG"; exit 1; }
echo -e "${GREEN}mock-actrix ready on http://127.0.0.1:$MOCK_PORT${NC}"

# ---- Step 4: seed realm + MFR + packages ----

echo ""
echo -e "${BLUE}Step 4: Seeding realm + MFR + packages on mock-actrix...${NC}"

ENDPOINT="http://127.0.0.1:$MOCK_PORT"
bash "$SCRIPT_DIR/register-mock.sh" --endpoint "$ENDPOINT"

echo -e "${GREEN}Registration complete${NC}"

# ---- Step 5: `actr run --web` with WBG mode ----

echo ""
echo -e "${BLUE}Step 5: Starting actr run --web (WBG mode, server + client)...${NC}"

# The magic selector: makes `serve_actor_sw_js` return ACTOR_WBG_SW_JS.
export ACTR_WEB_GUEST_MODE=wbg

"$ACTR_CMD" run --web -c "$SERVER_ACTR_TOML" > "$LOG_DIR/server.log" 2>&1 &
SERVER_PID=$!
echo "  Server started (PID: $SERVER_PID) on port 5174 (ACTR_WEB_GUEST_MODE=wbg)"

"$ACTR_CMD" run --web -c "$CLIENT_ACTR_TOML" > "$LOG_DIR/client.log" 2>&1 &
CLIENT_PID=$!
echo "  Client started (PID: $CLIENT_PID) on port 5173 (ACTR_WEB_GUEST_MODE=wbg)"

for PORT in 5173 5174; do
    READY=0
    for _ in $(seq 1 60); do
        if lsof -i:"$PORT" >/dev/null 2>&1 || nc -z 127.0.0.1 "$PORT" 2>/dev/null; then
            READY=1; break
        fi
        sleep 0.1
    done
    [ "$READY" -eq 1 ] || { echo -e "${RED}port $PORT not bound within 6s${NC}"; cat "$LOG_DIR/server.log" "$LOG_DIR/client.log" 2>/dev/null || true; exit 1; }
done
echo -e "${GREEN}Server at http://localhost:5174, client at http://localhost:5173${NC}"

# ---- Step 6: run automated test ----

echo ""
echo -e "${BLUE}Step 6: Running automated test (CAPTURE_SW_CONSOLE=1)...${NC}"

TEST_EXIT_CODE=-1
if [ -f "$SCRIPT_DIR/test-auto.js" ]; then
    if ! node -e "require('puppeteer')" 2>/dev/null; then
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
        [ -n "$CHROME_PATH" ] && export PUPPETEER_EXECUTABLE_PATH="$CHROME_PATH"
    fi

    set +e
    CLIENT_URL="http://localhost:5173" \
    SERVER_URL="http://localhost:5174" \
    CAPTURE_SW_CONSOLE=1 \
    node "$SCRIPT_DIR/test-auto.js" BasicFunction
    TEST_EXIT_CODE=$?
    set -e
else
    echo -e "${YELLOW}test-auto.js not found, skipping${NC}"
fi

# ---- Summary ----

echo ""
echo "Services:"
echo "  mock-actrix: http://127.0.0.1:$MOCK_PORT"
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
