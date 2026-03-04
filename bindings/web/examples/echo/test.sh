#!/bin/bash
# Echo Example — Automated Test Runner
#
# Validates that required services are running, installs puppeteer if needed,
# then launches test-auto.js with the correct environment.
#
# Usage:
#   ./test.sh                    # Run all suites
#   ./test.sh MultiTab           # Run only MultiTab suite
#   ./test.sh MultiTab Webrtc    # Run multiple suites
#   SLOW=1 ./test.sh             # Include slow tests
#   RUN_C=1 ./test.sh            # Include orchestration tests

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR="/tmp/echo-test"

# Default service URLs
: "${CLIENT_URL:=https://localhost:5173}"
: "${SERVER_URL:=http://localhost:5174}"
: "${SIGNALING_PORT:=8081}"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_error()   { echo -e "${RED}[ERROR]${NC} $1"; }
log_warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }

print_banner() {
    echo ""
    echo "╔════════════════════════════════════════════════════════════╗"
    echo "║   🧪 Echo — A+B+C Category Automated Test Runner         ║"
    echo "╚════════════════════════════════════════════════════════════╝"
    echo ""
}

# Extract port from URL
url_port() {
    echo "$1" | sed -E 's|.*:([0-9]+).*|\1|'
}

# Check that required services are running
check_services() {
    local client_port server_port
    client_port=$(url_port "$CLIENT_URL")
    server_port=$(url_port "$SERVER_URL")

    local missing=0

    if ! lsof -iTCP:"$client_port" -sTCP:LISTEN -t >/dev/null 2>&1; then
        log_error "Echo client not running on port $client_port ($CLIENT_URL)"
        missing=1
    else
        log_success "Echo client: port $client_port"
    fi

    if ! lsof -iTCP:"$server_port" -sTCP:LISTEN -t >/dev/null 2>&1; then
        log_error "Echo server not running on port $server_port ($SERVER_URL)"
        missing=1
    else
        log_success "Echo server: port $server_port"
    fi

    if ! lsof -iTCP:"$SIGNALING_PORT" -sTCP:LISTEN -t >/dev/null 2>&1; then
        log_error "Actrix signaling not running on port $SIGNALING_PORT"
        missing=1
    else
        log_success "Actrix signaling: port $SIGNALING_PORT"
    fi

    if [ "$missing" -eq 1 ]; then
        echo ""
        log_error "Some services are not running. Start them first:"
        echo "  1. actrix (signaling)   — cd ../../../../actrix && cargo run"
        echo "  2. echo server          — cd server && pnpm dev"
        echo "  3. echo client          — cd client && pnpm dev"
        exit 1
    fi
    echo ""
}

# Ensure puppeteer is installed in the test directory
ensure_puppeteer() {
    if [ ! -d "$TEST_DIR/node_modules/puppeteer" ]; then
        log_info "Installing puppeteer in $TEST_DIR..."
        mkdir -p "$TEST_DIR"
        (cd "$TEST_DIR" && npm init -y --silent 2>/dev/null && npm install puppeteer --silent 2>&1 | tail -1)
        log_success "Puppeteer installed"
    else
        log_success "Puppeteer ready ($TEST_DIR)"
    fi
    echo ""
}

# Run the test suite
run_tests() {
    log_info "Running test-auto.js..."
    [ -n "${SLOW:-}" ] && log_info "SLOW tests: enabled"
    [ -n "${RUN_C:-}" ] && log_info "C-category tests: enabled"
    if [ $# -gt 0 ]; then
        log_info "Suite filter: $*"
    fi
    echo ""

    cd "$SCRIPT_DIR"

    NODE_PATH="$TEST_DIR/node_modules" \
    CLIENT_URL="$CLIENT_URL" \
    SERVER_URL="$SERVER_URL" \
    SLOW="${SLOW:-}" \
    RUN_C="${RUN_C:-}" \
    node test-auto.js "$@"
}

main() {
    print_banner
    check_services
    ensure_puppeteer
    run_tests "$@"
}

main "$@"
