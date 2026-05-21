#!/bin/bash
# Actor-RTC Web Data-Stream Peer Concurrent Example Launcher
# Uses the in-repo mock-actrix so the example does not depend on an external
# actrix checkout, sqlite schema, or pre-seeded manufacturer state.

set -e  # Exit on error

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"

CLIENT_DIR="$SCRIPT_DIR/client"
SERVER_DIR="$SCRIPT_DIR/server"
MOCK_PORT="${1:-8081}"
ACTRIX_HTTP_URL="http://127.0.0.1:$MOCK_PORT"
ACTRIX_SIGNALING_URL="ws://127.0.0.1:$MOCK_PORT/signaling/ws"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
NC='\033[0m'

log_info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
log_warning() { echo -e "${YELLOW}[WARNING]${NC} $1"; }
log_error()   { echo -e "${RED}[ERROR]${NC} $1"; }
log_step()    { echo -e "${MAGENTA}[STEP]${NC} $1"; }

command_exists() { command -v "$1" >/dev/null 2>&1; }

# ── Cleanup ──

cleanup() {
    echo ""
    log_info "Shutting down services..."

    if [ -f "$SCRIPT_DIR/.client.pid" ]; then
        PID=$(cat "$SCRIPT_DIR/.client.pid")
        kill -0 $PID 2>/dev/null && kill $PID 2>/dev/null || true
        rm -f "$SCRIPT_DIR/.client.pid"
    fi

    if [ -f "$SCRIPT_DIR/.client2.pid" ]; then
        PID=$(cat "$SCRIPT_DIR/.client2.pid")
        kill -0 $PID 2>/dev/null && kill $PID 2>/dev/null || true
        rm -f "$SCRIPT_DIR/.client2.pid"
    fi

    if [ -f "$SCRIPT_DIR/.server.pid" ]; then
        PID=$(cat "$SCRIPT_DIR/.server.pid")
        kill -0 $PID 2>/dev/null && kill $PID 2>/dev/null || true
        rm -f "$SCRIPT_DIR/.server.pid"
    fi

    if [ -f "$SCRIPT_DIR/.mock-actrix.pid" ]; then
        PID=$(cat "$SCRIPT_DIR/.mock-actrix.pid")
        kill -0 $PID 2>/dev/null && kill $PID 2>/dev/null || true
        rm -f "$SCRIPT_DIR/.mock-actrix.pid"
    fi

    wait 2>/dev/null || true
    log_success "Cleanup complete"
}

trap cleanup EXIT INT TERM

# ── Dependency check ──

check_dependencies() {
    log_step "Checking dependencies..."
    local missing=0

    if command_exists node; then
        log_success "Node.js: $(node --version)"
    else
        log_error "Node.js not found"; missing=1
    fi

    if command_exists pnpm; then
        log_success "pnpm: v$(pnpm --version)"
    else
        log_error "pnpm not found"; missing=1
    fi

    [ $missing -eq 1 ] && exit 1
    echo ""
}

# ── mock-actrix (signaling + AIS server) ──

build_or_find_mock_actrix() {
    log_step "Checking mock-actrix (signaling + AIS server)..."
    MOCK_ACTRIX_CMD=""

    if [ -x "$ACTR_ROOT/target/debug/mock-actrix" ]; then
        MOCK_ACTRIX_CMD="$ACTR_ROOT/target/debug/mock-actrix"
        log_success "Using local mock-actrix: $MOCK_ACTRIX_CMD"
        return 0
    fi
    if [ -x "$ACTR_ROOT/target/release/mock-actrix" ]; then
        MOCK_ACTRIX_CMD="$ACTR_ROOT/target/release/mock-actrix"
        log_success "Using local mock-actrix: $MOCK_ACTRIX_CMD"
        return 0
    fi

    log_info "Building mock-actrix..."
    (cd "$ACTR_ROOT" && cargo build -p actr-mock-actrix --bin mock-actrix 2>&1 | tail -5)
    if [ -x "$ACTR_ROOT/target/debug/mock-actrix" ]; then
        MOCK_ACTRIX_CMD="$ACTR_ROOT/target/debug/mock-actrix"
        log_success "Built mock-actrix: $MOCK_ACTRIX_CMD"
        return 0
    fi

    log_error "mock-actrix not found"
    return 1
}

start_mock_actrix() {
    log_step "Starting mock-actrix on port $MOCK_PORT..."

    if ! build_or_find_mock_actrix; then
        exit 1
    fi

    if lsof -ti:"$MOCK_PORT" >/dev/null 2>&1; then
        log_error "Port $MOCK_PORT is already in use. Stop that process before running the example."
        exit 1
    fi

    "$MOCK_ACTRIX_CMD" --port "$MOCK_PORT" > "$SCRIPT_DIR/mock-actrix.log" 2>&1 &
    MOCK_ACTRIX_PID=$!
    echo $MOCK_ACTRIX_PID > "$SCRIPT_DIR/.mock-actrix.pid"

    log_success "mock-actrix started (PID: $MOCK_ACTRIX_PID)"
    log_info "Waiting for mock-actrix to be ready..."

    local ready=0
    for _ in $(seq 1 100); do
        if ! kill -0 "$MOCK_ACTRIX_PID" 2>/dev/null; then
            log_error "mock-actrix failed to start"
            cat "$SCRIPT_DIR/mock-actrix.log"
            exit 1
        fi
        if curl -fsS "$ACTRIX_HTTP_URL/health" >/dev/null 2>&1; then
            ready=1
            break
        fi
        sleep 0.1
    done
    if [ "$ready" -ne 1 ]; then
        log_error "mock-actrix did not become healthy"
        cat "$SCRIPT_DIR/mock-actrix.log"
        exit 1
    fi

    log_success "mock-actrix is running at $ACTRIX_HTTP_URL"
    echo ""
}

# ── Realm setup ──

setup_realm() {
    log_step "Setting up realm (AIS identity)..."

    # realm_id is hardcoded in config.ts files
    local REALM_ID=2368266035

    log_info "Creating realm $REALM_ID..."
    curl -fsS -X POST "$ACTRIX_HTTP_URL/admin/realms" \
        -H 'content-type: application/json' \
        --data "{\"id\": $REALM_ID, \"name\": \"data-stream-realm\"}" >/dev/null

    log_success "Realm setup complete (realm_id=$REALM_ID)"
    echo ""
}

# ── WASM build ──

build_wasm() {
    log_step "Checking WASM artifacts..."

    if [ -f "$SERVER_DIR/public/data_stream_server_bg.wasm" ] && [ -f "$SERVER_DIR/public/data_stream_server.js" ]; then
        log_success "Server WASM already built"
    else
        log_info "Building server WASM..."
        cd "$SERVER_DIR"
        bash build.sh 2>&1 | tee "$SCRIPT_DIR/wasm-server-build.log"
        log_success "Server WASM built"
    fi

    if [ -f "$CLIENT_DIR/public/data_stream_client_bg.wasm" ] && [ -f "$CLIENT_DIR/public/data_stream_client.js" ]; then
        log_success "Client WASM already built"
    else
        log_info "Building client WASM..."
        cd "$CLIENT_DIR"
        bash build.sh 2>&1 | tee "$SCRIPT_DIR/wasm-client-build.log"
        log_success "Client WASM built"
    fi

    echo ""
}

# ── Install web deps ──

install_deps() {
    log_step "Installing web dependencies..."
    cd "$PROJECT_ROOT"
    log_info "Running pnpm install at web workspace root..."
    pnpm install 2>&1 | tail -5
    log_success "Dependencies installed"
    echo ""
}

# ── Start dev servers ──

start_server() {
    log_step "Starting Data-Stream Server..."
    cd "$SERVER_DIR"
    VITE_ACTRIX_HTTP_URL="$ACTRIX_HTTP_URL" \
    VITE_ACTRIX_SIGNALING_URL="$ACTRIX_SIGNALING_URL" \
    pnpm dev --host 127.0.0.1 --port 4176 > "$SCRIPT_DIR/server.log" 2>&1 &
    SERVER_PID=$!
    echo $SERVER_PID > "$SCRIPT_DIR/.server.pid"
    log_success "Server dev started (PID: $SERVER_PID)"
    sleep 3

    if ! kill -0 $SERVER_PID 2>/dev/null; then
        log_error "Server failed to start"
        cat "$SCRIPT_DIR/server.log"
        exit 1
    fi
    log_success "Server is running at http://localhost:4176"
    echo ""
}

start_client() {
    # Start TWO client Vite dev servers on separate ports.
    # Each port = separate origin = separate Service Worker = separate actor_id.
    # This avoids the signaling WS conflict where a second SW registration with
    # the same actor_id silently kills the first connection.
    log_step "Starting Data-Stream Clients (2 instances on different ports)..."

    cd "$CLIENT_DIR"

    # Client-1 on port 4175
    VITE_ACTRIX_HTTP_URL="$ACTRIX_HTTP_URL" \
    VITE_ACTRIX_SIGNALING_URL="$ACTRIX_SIGNALING_URL" \
    pnpm dev --host 127.0.0.1 --port 4175 > "$SCRIPT_DIR/client.log" 2>&1 &
    CLIENT_PID=$!
    echo $CLIENT_PID > "$SCRIPT_DIR/.client.pid"
    log_success "Client-1 dev started (PID: $CLIENT_PID, port 4175)"

    # Client-2 on port 4177
    VITE_ACTRIX_HTTP_URL="$ACTRIX_HTTP_URL" \
    VITE_ACTRIX_SIGNALING_URL="$ACTRIX_SIGNALING_URL" \
    pnpm dev --host 127.0.0.1 --port 4177 > "$SCRIPT_DIR/client2.log" 2>&1 &
    CLIENT2_PID=$!
    echo $CLIENT2_PID > "$SCRIPT_DIR/.client2.pid"
    log_success "Client-2 dev started (PID: $CLIENT2_PID, port 4177)"

    sleep 3

    if ! kill -0 $CLIENT_PID 2>/dev/null; then
        log_error "Client-1 failed to start"
        cat "$SCRIPT_DIR/client.log"
        exit 1
    fi
    if ! kill -0 $CLIENT2_PID 2>/dev/null; then
        log_error "Client-2 failed to start"
        cat "$SCRIPT_DIR/client2.log"
        exit 1
    fi
    log_success "Client-1 is running at http://localhost:4175"
    log_success "Client-2 is running at http://localhost:4177"
    echo ""
}

# ── Automated test ──

run_test() {
    log_step "Running automated browser test..."

    cd "$SCRIPT_DIR"

    # Resolve puppeteer
    if node -e "require('puppeteer')" 2>/dev/null; then
        log_success "Puppeteer available"
    else
        local E2E_MODULES="$PROJECT_ROOT/tests/e2e/node_modules"
        if [ -d "$E2E_MODULES/puppeteer" ]; then
            export NODE_PATH="$E2E_MODULES:${NODE_PATH:-}"
            log_success "Puppeteer found via workspace tests/e2e"
        else
            log_info "Installing puppeteer via pnpm..."
            cd "$PROJECT_ROOT"
            PUPPETEER_SKIP_DOWNLOAD=true pnpm add -Dw puppeteer 2>&1 | tail -3
            cd "$SCRIPT_DIR"
            export NODE_PATH="$PROJECT_ROOT/node_modules:${NODE_PATH:-}"
        fi
    fi

    # Use system Chrome if needed
    if ! node -e "require('puppeteer').launch({headless:'new'}).then(b=>b.close())" 2>/dev/null; then
        local CHROME_PATH=""
        if [ -f "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" ]; then
            CHROME_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        elif command_exists google-chrome; then
            CHROME_PATH="$(which google-chrome)"
        elif command_exists chromium; then
            CHROME_PATH="$(which chromium)"
        fi
        if [ -n "$CHROME_PATH" ]; then
            export PUPPETEER_EXECUTABLE_PATH="$CHROME_PATH"
            log_success "Using system Chrome: $CHROME_PATH"
        else
            log_error "No Chrome/Chromium found for puppeteer"
            return 1
        fi
    fi

    # Give services time to stabilize
    log_info "Waiting for services to stabilize..."
    sleep 5

    log_info "Executing: node test-auto.js"
    echo ""

    set +e
    CLIENT_URLS="http://127.0.0.1:4175,http://127.0.0.1:4177" \
    SERVER_URL="http://127.0.0.1:4176" \
    node "$SCRIPT_DIR/test-auto.js"
    TEST_EXIT_CODE=$?
    set -e

    echo ""
    if [ $TEST_EXIT_CODE -eq 0 ]; then
        log_success "🎉 Data-Stream Peer Concurrent tests PASSED!"
    else
        log_error "Tests FAILED (exit code: $TEST_EXIT_CODE)"
        log_info "Service logs:"
        echo "  tail -f $SCRIPT_DIR/mock-actrix.log"
        echo "  tail -f $SCRIPT_DIR/server.log"
        echo "  tail -f $SCRIPT_DIR/client.log"
        echo "  tail -f $SCRIPT_DIR/client2.log"
    fi

    return $TEST_EXIT_CODE
}

# ── Main ──

main() {
    echo ""
    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║   🚀 Actor-RTC Web - Data Stream Peer Concurrent        ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo ""

    check_dependencies
    start_mock_actrix
    setup_realm
    build_wasm
    install_deps
    start_server
    start_client

    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║         ✅ Data-Stream Example Running                   ║"
    echo "╠═══════════════════════════════════════════════════════════╣"
    printf "║  mock-actrix: %-43s ║\n" "$ACTRIX_HTTP_URL"
    echo "║  Server:     Data-stream at http://localhost:4176       ║"
    echo "║  Client-1:   Web UI at http://localhost:4175            ║"
    echo "║  Client-2:   Web UI at http://localhost:4177            ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo ""

    run_test
    TEST_RESULT=$?

    echo ""
    if [ $TEST_RESULT -eq 0 ]; then
        log_success "✅ Data-stream peer concurrent verified end-to-end!"
    else
        log_error "❌ Verification failed"
        log_info "Services still running for debugging."
        log_info "  mock-actrix: $ACTRIX_HTTP_URL"
        log_info "  Server:   http://localhost:4176"
        log_info "  Client-1: http://localhost:4175"
        log_info "  Client-2: http://localhost:4177"
        log_info "Press Ctrl+C to stop all services"
        wait
    fi

    exit $TEST_RESULT
}

main
