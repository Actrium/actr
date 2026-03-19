#!/bin/bash
# Actor-RTC Web Data-Stream Peer Concurrent Example Launcher
# Based on the working echo example start.sh

set -e  # Exit on error

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"
ACTRIX_DIR="$(cd "$ACTR_ROOT/../actrix" && pwd)"

CLIENT_DIR="$SCRIPT_DIR/client"
SERVER_DIR="$SCRIPT_DIR/server"

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

    if [ -f "$SCRIPT_DIR/.actrix.pid" ]; then
        PID=$(cat "$SCRIPT_DIR/.actrix.pid")
        kill -0 $PID 2>/dev/null && kill $PID 2>/dev/null || true
        rm -f "$SCRIPT_DIR/.actrix.pid"
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

# ── Actrix (signaling server) ──

build_or_find_actrix() {
    log_step "Checking actrix (signaling server)..."
    ACTRIX_CMD=""

    if [ -f "$ACTRIX_DIR/target/release/actrix" ]; then
        ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
        log_success "Using local actrix (release): $ACTRIX_CMD"
        return 0
    fi
    if [ -f "$ACTRIX_DIR/target/debug/actrix" ]; then
        ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
        log_success "Using local actrix (debug): $ACTRIX_CMD"
        return 0
    fi
    if command_exists actrix; then
        ACTRIX_CMD="actrix"
        log_warning "Using installed actrix: $(which actrix)"
        return 0
    fi

    log_error "actrix not found"
    log_info "Build: cd $ACTRIX_DIR && cargo build"
    return 1
}

start_actrix() {
    log_step "Starting actrix (signaling server)..."

    if ! build_or_find_actrix; then
        exit 1
    fi

    cat > "$SCRIPT_DIR/actrix-dev.toml" <<'EOF'
# Auto-generated dev config for data-stream-peer-concurrent example
enable = 31
name = "actrix-data-stream-dev"
env = "dev"
sqlite_path = "actrix-dev-db"
location_tag = "local,dev,default"
actrix_shared_key = "data-stream-dev-secret-key-9876543210abcdef"

[recording]
service_name = "actrix-data-stream-dev"

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

[control.grpc_api]
node_id = "actrix-data-stream-dev"
node_name = "actrix-data-stream-dev"
shared_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
max_clock_skew_secs = 300

[acl]
enabled = true
default_policy = "allow"
EOF

    $ACTRIX_CMD --config "$SCRIPT_DIR/actrix-dev.toml" > "$SCRIPT_DIR/actrix.log" 2>&1 &
    ACTRIX_PID=$!
    echo $ACTRIX_PID > "$SCRIPT_DIR/.actrix.pid"

    log_success "Actrix started (PID: $ACTRIX_PID)"
    log_info "Waiting for actrix to be ready..."
    sleep 3

    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        log_error "Actrix failed to start"
        cat "$SCRIPT_DIR/actrix.log"
        exit 1
    fi

    log_success "Actrix is running at http://localhost:8081"
    echo ""
}

# ── Realm setup ──

setup_realm() {
    log_step "Setting up realm (AIS identity)..."
    sleep 2

    # realm_id is hardcoded in config.ts files
    local REALM_ID=2368266035
    local ACTRIX_DB="$SCRIPT_DIR/actrix-dev-db/actrix.db"

    if [ ! -f "$ACTRIX_DB" ]; then
        log_error "Actrix database not found at $ACTRIX_DB"
        exit 1
    fi

    log_info "Creating realm $REALM_ID..."
    sqlite3 "$ACTRIX_DB" \
        "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'data-stream-realm', 'Active', 1, strftime('%s','now'), '');"

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
    pnpm dev > "$SCRIPT_DIR/server.log" 2>&1 &
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
    pnpm dev --port 4175 > "$SCRIPT_DIR/client.log" 2>&1 &
    CLIENT_PID=$!
    echo $CLIENT_PID > "$SCRIPT_DIR/.client.pid"
    log_success "Client-1 dev started (PID: $CLIENT_PID, port 4175)"

    # Client-2 on port 4177
    pnpm dev --port 4177 > "$SCRIPT_DIR/client2.log" 2>&1 &
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
        echo "  tail -f $SCRIPT_DIR/actrix.log"
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
    start_actrix
    setup_realm
    build_wasm
    install_deps
    start_server
    start_client

    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║         ✅ Data-Stream Example Running                   ║"
    echo "╠═══════════════════════════════════════════════════════════╣"
    echo "║  Actrix:     Signaling at http://localhost:8081         ║"
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
        log_info "  Server:   http://localhost:4176"
        log_info "  Client-1: http://localhost:4175"
        log_info "  Client-2: http://localhost:4177"
        log_info "Press Ctrl+C to stop all services"
        wait
    fi

    exit $TEST_RESULT
}

main
