#!/bin/bash
# Actor-RTC Web Echo Example - Real Implementation Launcher
# 100% Real: Actor-RTC + WebRTC + WASM + IndexedDB

set -e  # Exit on error

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# actr repo root (parent of bindings/web)
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"
ACTRIX_DIR="$(cd "$ACTR_ROOT/../actrix" && pwd)"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${MAGENTA}[STEP]${NC} $1"
}

print_banner() {
    echo ""
    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║   🚀 Actor-RTC Web - Real Echo Implementation           ║"
    echo "║   100% Real: No Mocks, No Fakes                          ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo ""
}

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Cleanup function
cleanup() {
    echo ""
    log_info "Shutting down services..."

    # Kill web client
    if [ -f "$SCRIPT_DIR/.client.pid" ]; then
        CLIENT_PID=$(cat "$SCRIPT_DIR/.client.pid")
        if kill -0 $CLIENT_PID 2>/dev/null; then
            log_info "Stopping web client (PID: $CLIENT_PID)"
            kill $CLIENT_PID 2>/dev/null || true
        fi
        rm "$SCRIPT_DIR/.client.pid"
    fi

    # Kill Actor-RTC server
    if [ -f "$SCRIPT_DIR/.server.pid" ]; then
        SERVER_PID=$(cat "$SCRIPT_DIR/.server.pid")
        if kill -0 $SERVER_PID 2>/dev/null; then
            log_info "Stopping Actor-RTC server (PID: $SERVER_PID)"
            kill $SERVER_PID 2>/dev/null || true
        fi
        rm "$SCRIPT_DIR/.server.pid"
    fi

    # Kill actrix
    if [ -f "$SCRIPT_DIR/.actrix.pid" ]; then
        ACTRIX_PID=$(cat "$SCRIPT_DIR/.actrix.pid")
        if kill -0 $ACTRIX_PID 2>/dev/null; then
            log_info "Stopping actrix (PID: $ACTRIX_PID)"
            kill $ACTRIX_PID 2>/dev/null || true
        fi
        rm "$SCRIPT_DIR/.actrix.pid"
    fi

    wait 2>/dev/null || true
    log_success "Cleanup complete"
    echo ""
}

trap cleanup EXIT INT TERM

check_dependencies() {
    log_step "Checking dependencies..."
    echo ""

    local missing=0

    # Node.js
    if command_exists node; then
        log_success "Node.js: $(node --version)"
    else
        log_error "Node.js not found - Install from https://nodejs.org/"
        missing=1
    fi

    # pnpm
    if command_exists pnpm; then
        log_success "pnpm: v$(pnpm --version)"
    else
        log_error "pnpm not found - Install: npm install -g pnpm"
        missing=1
    fi

    if [ $missing -eq 1 ]; then
        log_error "Missing required dependencies"
        exit 1
    fi

    echo ""
}

build_or_find_actrix() {
    log_step "Checking actrix (signaling server)..."

    ACTRIX_CMD=""

    # Check for local builds first (prioritize local over installed)
    if [ -f "$ACTRIX_DIR/target/release/actrix" ]; then
        ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
        log_success "Using local actrix build (release): $ACTRIX_CMD"
        return 0
    fi

    if [ -f "$ACTRIX_DIR/target/debug/actrix" ]; then
        ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
        log_success "Using local actrix build (debug): $ACTRIX_CMD"
        return 0
    fi

    # Check if actrix is installed
    if command_exists actrix; then
        ACTRIX_CMD="actrix"
        log_warning "Using installed actrix: $(which actrix)"
        log_warning "Note: Consider using local build for latest features"
        return 0
    fi

    # Not found - provide installation instructions
    log_error "actrix not found"
    log_info "Install actrix using one of these methods:"
    log_info "  1. cd $ACTRIX_DIR && cargo build  (recommended - build from source)"
    log_info "  2. cargo install actrix  (install from crates.io)"
    log_info ""
    return 1
}

start_actrix() {
    log_step "Starting actrix (signaling server)..."

    if ! build_or_find_actrix; then
        exit 1
    fi

    # Create development config with all services enabled
    cat > "$SCRIPT_DIR/actrix-dev.toml" <<'EOF'
# Auto-generated dev config for web echo example
enable = 31
name = "actrix-web-echo-dev"
env = "dev"
sqlite_path = "actrix-dev-db"
location_tag = "local,dev,default"
actrix_shared_key = "web-echo-dev-secret-key-9876543210abcdef"

[recording]
service_name = "actrix-web-echo-dev"

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
node_id = "actrix-web-echo-dev"
node_name = "actrix-web-echo-dev"
shared_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
max_clock_skew_secs = 300

[acl]
enabled = true
default_policy = "allow"
EOF

    # Start actrix
    $ACTRIX_CMD --config "$SCRIPT_DIR/actrix-dev.toml" > "$SCRIPT_DIR/actrix.log" 2>&1 &
    ACTRIX_PID=$!
    echo $ACTRIX_PID > "$SCRIPT_DIR/.actrix.pid"

    log_success "Actrix started (PID: $ACTRIX_PID)"
    log_info "Actrix logs: $SCRIPT_DIR/actrix.log"
    log_info "Waiting for actrix to be ready..."
    sleep 3

    # Check if actrix is still running
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        log_error "Actrix failed to start"
        log_info "Check logs: cat $SCRIPT_DIR/actrix.log"
        cat "$SCRIPT_DIR/actrix.log"
        exit 1
    fi

    log_success "Actrix is running at http://localhost:8081"
    echo ""
}

setup_realm() {
    log_step "Setting up realm (AIS identity)..."

    local SERVER_DIR="$SCRIPT_DIR/server"
    local CLIENT_DIR="$SCRIPT_DIR/client"

    sleep 2

    # Extract realm IDs from actr.toml files
    local SERVER_REALM
    SERVER_REALM=$(grep -E 'realm_id\s*=' "$SERVER_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
    local CLIENT_REALM
    CLIENT_REALM=$(grep -E 'realm_id\s*=' "$CLIENT_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')

    log_info "Server realm_id=$SERVER_REALM, Client realm_id=$CLIENT_REALM"

    # Insert realms directly into SQLite (same approach as actrix fullstack tests)
    local ACTRIX_DB="$SCRIPT_DIR/actrix-dev-db/actrix.db"

    if [ ! -f "$ACTRIX_DB" ]; then
        log_error "Actrix database not found at $ACTRIX_DB"
        log_info "Actrix may not have started properly."
        exit 1
    fi

    for REALM_ID in $SERVER_REALM $CLIENT_REALM; do
        log_info "Creating realm $REALM_ID..."
        sqlite3 "$ACTRIX_DB" \
            "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'wasm-echo-realm', 'Active', 1, strftime('%s','now'), '');"
    done

    log_success "Realms setup completed (realm IDs: $SERVER_REALM, $CLIENT_REALM)"

    # Patch actr-config.ts in both client and server with the correct realm_id
    log_info "Patching config files with realm_id from actr.toml..."

    local CLIENT_CONFIG="$SCRIPT_DIR/client/src/generated/actr-config.ts"
    local SERVER_CONFIG="$SCRIPT_DIR/server/src/generated/actr-config.ts"

    if [ -f "$SERVER_CONFIG" ]; then
        sed -i '' "s/realm_id: [0-9][0-9]*,/realm_id: $SERVER_REALM,/g" "$SERVER_CONFIG"
        log_success "Patched server actr-config.ts with realm_id=$SERVER_REALM"
    fi

    if [ -f "$CLIENT_CONFIG" ]; then
        sed -i '' "s/realm_id: [0-9][0-9]*,/realm_id: $CLIENT_REALM,/g" "$CLIENT_CONFIG"
        log_success "Patched client actr-config.ts with realm_id=$CLIENT_REALM"
    fi

    log_success "Realm setup complete"
    echo ""
}

build_wasm() {
    log_step "Checking WASM artifacts..."

    # Check if server WASM is already built
    if [ -f "$SCRIPT_DIR/server/public/echo_server_bg.wasm" ] && [ -f "$SCRIPT_DIR/server/public/echo_server.js" ]; then
        log_success "Server WASM already built"
    else
        log_info "Building server WASM..."
        cd "$SCRIPT_DIR/server"
        bash build.sh 2>&1 | tee "$SCRIPT_DIR/wasm-server-build.log"
        log_success "Server WASM built"
    fi

    # Check if client WASM is already built
    if [ -f "$SCRIPT_DIR/client/public/echo_client_bg.wasm" ] && [ -f "$SCRIPT_DIR/client/public/echo_client.js" ]; then
        log_success "Client WASM already built"
    else
        log_info "Building client WASM..."
        cd "$SCRIPT_DIR/client"
        bash build.sh 2>&1 | tee "$SCRIPT_DIR/wasm-client-build.log"
        log_success "Client WASM built"
    fi

    echo ""
}

install_deps() {
    log_step "Installing web dependencies..."

    cd "$PROJECT_ROOT"

    # Always run pnpm install to ensure all workspace packages have node_modules
    log_info "Running pnpm install at web workspace root..."
    pnpm install 2>&1 | tail -5
    log_success "Dependencies installed"

    echo ""
}

start_server() {
    log_step "Starting Echo Server (browser-hosted)..."

    cd "$SCRIPT_DIR/server"

    pnpm dev > "$SCRIPT_DIR/server.log" 2>&1 &
    SERVER_PID=$!
    echo $SERVER_PID > "$SCRIPT_DIR/.server.pid"

    log_success "Echo Server dev server started (PID: $SERVER_PID)"
    log_info "Server logs: $SCRIPT_DIR/server.log"

    # Wait for vite to start
    log_info "Waiting for Vite dev server to start..."
    sleep 3

    # Check if server is still running
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        log_error "Server failed to start"
        log_info "Check logs: cat $SCRIPT_DIR/server.log"
        cat "$SCRIPT_DIR/server.log"
        exit 1
    fi

    log_success "Echo Server is running at http://localhost:5174"
    echo ""
}

start_client() {
    log_step "Starting Echo Client..."

    cd "$SCRIPT_DIR/client"

    pnpm dev > "$SCRIPT_DIR/client.log" 2>&1 &
    CLIENT_PID=$!
    echo $CLIENT_PID > "$SCRIPT_DIR/.client.pid"

    log_success "Client started (PID: $CLIENT_PID)"
    log_info "Client logs: $SCRIPT_DIR/client.log"

    # Wait for client dev server
    sleep 3

    # Check if client is still running
    if ! kill -0 $CLIENT_PID 2>/dev/null; then
        log_error "Client failed to start"
        log_info "Check logs: cat $SCRIPT_DIR/client.log"
        cat "$SCRIPT_DIR/client.log"
        exit 1
    fi

    log_success "Echo Client is running at https://localhost:5173"
    echo ""
}

open_browser() {
    local url="https://localhost:5173"

    log_info "Opening browser..."

    if command_exists xdg-open; then
        xdg-open "$url" 2>/dev/null
    elif command_exists open; then
        open "$url" 2>/dev/null
    elif command_exists wslview; then
        wslview "$url" 2>/dev/null
    else
        log_warning "Could not open browser automatically"
    fi

    echo ""
}

run_basic_test() {
    log_step "Running BasicFunction automated test..."

    cd "$SCRIPT_DIR"

    # Resolve puppeteer: try direct, then from tests/e2e workspace package
    local EXTRA_ENV=""
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

    # Use system Chrome if puppeteer's bundled Chrome isn't available
    if ! node -e "require('puppeteer').launch({headless:'new'}).then(b=>b.close())" 2>/dev/null; then
        local CHROME_PATH=""
        if [ -f "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" ]; then
            CHROME_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        elif command_exists google-chrome; then
            CHROME_PATH="$(which google-chrome)"
        elif command_exists chromium-browser; then
            CHROME_PATH="$(which chromium-browser)"
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

    # Verify puppeteer can launch
    if ! node -e "require('puppeteer')" 2>/dev/null; then
        log_error "Puppeteer not available. Install with: pnpm add -Dw puppeteer"
        return 1
    fi

    # Give services a bit more time to stabilize before running tests
    log_info "Waiting for services to fully stabilize..."
    sleep 5

    # Run BasicFunction suite only
    log_info "Executing: node test-auto.js BasicFunction"
    echo ""

    set +e  # Don't exit on test failure
    CLIENT_URL="https://localhost:5173" \
    SERVER_URL="http://localhost:5174" \
    node "$SCRIPT_DIR/test-auto.js" BasicFunction
    TEST_EXIT_CODE=$?
    set -e

    echo ""
    if [ $TEST_EXIT_CODE -eq 0 ]; then
        log_success "🎉 BasicFunction tests PASSED! Echo messaging is working."
    else
        log_error "BasicFunction tests FAILED (exit code: $TEST_EXIT_CODE)"
        log_info "Check the test output above for details."
        log_info "Service logs:"
        echo "  tail -f $SCRIPT_DIR/actrix.log"
        echo "  tail -f $SCRIPT_DIR/server.log"
        echo "  tail -f $SCRIPT_DIR/client.log"
    fi

    return $TEST_EXIT_CODE
}

main() {
    print_banner

    check_dependencies

    start_actrix

    setup_realm

    build_wasm

    install_deps

    start_server

    start_client

    # Print status
    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║              ✅ Real Echo Implementation Running         ║"
    echo "╠═══════════════════════════════════════════════════════════╣"
    echo "║  Actrix:   Signaling at http://localhost:8081           ║"
    echo "║  Server:   Echo service at http://localhost:5174        ║"
    echo "║  Client:   Web UI at https://localhost:5173             ║"
    echo "║  WASM:     SW Runtime + User Workload                   ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo ""

    log_success "🎉 All services are running!"
    echo ""

    # Run automated test to verify echo messaging works
    run_basic_test
    TEST_RESULT=$?

    echo ""
    if [ $TEST_RESULT -eq 0 ]; then
        log_success "✅ Verification complete — echo messaging is working end-to-end!"
    else
        log_error "❌ Verification failed — echo messaging is NOT working."
        log_info "Services are still running for manual debugging."
        log_info "  Client: https://localhost:5173"
        log_info "  Server: http://localhost:5174"
        log_info "Press Ctrl+C to stop all services"
        wait
    fi

    # Cleanup happens via trap
    exit $TEST_RESULT
}

main
