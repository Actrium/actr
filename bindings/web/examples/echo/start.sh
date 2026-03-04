#!/bin/bash
# Actor-RTC Web Echo Example - Real Implementation Launcher
# 100% Real: Actor-RTC + WebRTC + WASM + IndexedDB

set -e  # Exit on error

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTOR_RTC_DIR="$(cd "$PROJECT_ROOT/.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"

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

    # Rust
    if command_exists cargo; then
        log_success "Rust: $(cargo --version | awk '{print $2}')"
    else
        log_error "Rust not found - Install from https://rustup.rs/"
        missing=1
    fi

    # Node.js
    if command_exists node; then
        log_success "Node.js: $(node --version)"
    else
        log_error "Node.js not found - Install from https://nodejs.org/"
        missing=1
    fi

    # npm
    if command_exists npm; then
        log_success "npm: v$(npm --version)"
    else
        log_error "npm not found"
        missing=1
    fi

    # wasm-pack
    if command_exists wasm-pack; then
        log_success "wasm-pack: $(wasm-pack --version | awk '{print $2}')"
    else
        log_warning "wasm-pack not found - Installing..."
        cargo install wasm-pack || { log_error "Failed to install wasm-pack"; exit 1; }
        log_success "wasm-pack installed"
    fi

    # protoc
    if command_exists protoc; then
        log_success "protoc: $(protoc --version | awk '{print $2}')"
    else
        log_error "protoc not found - Install Protocol Buffers compiler"
        log_info "  Ubuntu/Debian: sudo apt install -y protobuf-compiler"
        log_info "  macOS: brew install protobuf"
        log_info "  Or download from: https://github.com/protocolbuffers/protobuf/releases"
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

    # Create development config with Signaling enabled
    cat > "$SCRIPT_DIR/actrix-dev.toml" <<'EOF'
enable = 6
name = "actrix-dev"
env = "dev"
log_level = "info"
log_output = "file"
log_path = "logs/"
sqlite_path = "actrix-dev.db"
location_tag = "local,dev,default"
actrix_shared_key = "actr-web-echo-dev-secret-key-98765"

[services.signaling]
enabled = true

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

[turn]
advertised_ip = "127.0.0.1"
advertised_port = 3478
relay_port_range = "49152-49252"
realm = "localhost"
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

build_wasm() {
    log_step "Building WASM runtime..."

    cd "$PROJECT_ROOT"

    if bash ./scripts/build-wasm.sh 2>&1 | tee "$SCRIPT_DIR/wasm-build.log"; then
        log_success "WASM runtime built"
    else
        log_warning "WASM build completed with warnings (check wasm-build.log)"
    fi

    echo ""
}

generate_proto_client() {
    log_step "Generating TypeScript protobuf code..."

    cd "$SCRIPT_DIR/client"

    # Create generated directory
    mkdir -p src/generated

    # Check for protoc-gen-js plugin
    if ! command_exists protoc-gen-js; then
        log_error "protoc-gen-js not found - Required for JavaScript protobuf generation"
        log_info "Install via npm (recommended):"
        log_info "  npm install -g protoc-gen-js"
        log_info ""
        log_info "Or install grpc-tools which includes it:"
        log_info "  npm install -g grpc-tools"
        log_info ""
        log_info "Or ensure you have the latest protoc with built-in JavaScript support:"
        log_info "  https://github.com/protocolbuffers/protobuf/releases"
        exit 1
    fi

    # Check for grpc-web protoc plugin
    if ! command_exists protoc-gen-grpc-web; then
        log_warning "protoc-gen-grpc-web not found - Installing..."

        # Download grpc-web plugin
        local OS="linux"
        if [[ "$OSTYPE" == "darwin"* ]]; then
            OS="darwin"
        fi

        local PLUGIN_VERSION="1.5.0"
        local PLUGIN_URL="https://github.com/grpc/grpc-web/releases/download/${PLUGIN_VERSION}/protoc-gen-grpc-web-${PLUGIN_VERSION}-${OS}-x86_64"

        log_info "Downloading protoc-gen-grpc-web..."
        curl -L "$PLUGIN_URL" -o /tmp/protoc-gen-grpc-web
        chmod +x /tmp/protoc-gen-grpc-web
        sudo mv /tmp/protoc-gen-grpc-web /usr/local/bin/ || mv /tmp/protoc-gen-grpc-web ~/.local/bin/ || {
            log_error "Failed to install protoc-gen-grpc-web"
            exit 1
        }

        log_success "protoc-gen-grpc-web installed"
    fi

    # Generate JavaScript protobuf code
    protoc \
        --js_out=import_style=commonjs,binary:./src/generated \
        --grpc-web_out=import_style=typescript,mode=grpcwebtext:./src/generated \
        --proto_path=../proto \
        ../proto/echo.proto

    log_success "TypeScript protobuf code generated"
    echo ""
}

build_server() {
    log_step "Building Actor-RTC server..."

    cd "$SCRIPT_DIR/server"

    cargo build --release 2>&1 | tee "$SCRIPT_DIR/server-build.log"

    if [ ${PIPESTATUS[0]} -eq 0 ]; then
        log_success "Actor-RTC server built successfully"
    else
        log_error "Server build failed (see server-build.log)"
        exit 1
    fi

    echo ""
}

start_server() {
    log_step "Starting Actor-RTC server..."

    cd "$SCRIPT_DIR/server"

    RUST_LOG="${RUST_LOG:-info}" cargo run --release > "$SCRIPT_DIR/server.log" 2>&1 &
    SERVER_PID=$!
    echo $SERVER_PID > "$SCRIPT_DIR/.server.pid"

    log_success "Actor-RTC server started (PID: $SERVER_PID)"
    log_info "Server logs: $SCRIPT_DIR/server.log"

    # Wait for server to connect to signaling server
    log_info "Waiting for server to register with actrix..."
    sleep 3

    # Check if server is still running
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        log_error "Server failed to start"
        log_info "Check logs: cat $SCRIPT_DIR/server.log"
        cat "$SCRIPT_DIR/server.log"
        exit 1
    fi

    log_success "Actor-RTC server is running"
    echo ""
}

setup_client() {
    log_step "Setting up web client..."

    cd "$SCRIPT_DIR/client"

    if [ ! -d "node_modules" ]; then
        log_info "Installing dependencies..."
        npm install
        log_success "Dependencies installed"
    else
        log_success "Dependencies already installed"
    fi

    echo ""
}

start_client() {
    log_step "Starting web client..."

    cd "$SCRIPT_DIR/client"

    npm run dev > "$SCRIPT_DIR/client.log" 2>&1 &
    CLIENT_PID=$!
    echo $CLIENT_PID > "$SCRIPT_DIR/.client.pid"

    log_success "Client started (PID: $CLIENT_PID)"
    log_info "Client logs: $SCRIPT_DIR/client.log"

    # Wait for client
    sleep 3

    echo ""
}

open_browser() {
    local url="http://localhost:3000"

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

main() {
    print_banner

    check_dependencies

    start_actrix

    build_wasm

    generate_proto_client

    build_server

    start_server

    setup_client

    start_client

    open_browser

    # Print status
    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║              ✅ Real Echo Implementation Running         ║"
    echo "╠═══════════════════════════════════════════════════════════╣"
    echo "║  Actrix:   Signaling at http://localhost:8081           ║"
    echo "║  Server:   Actor-RTC via WebRTC (actr-runtime)          ║"
    echo "║  Client:   Web UI at http://localhost:3000              ║"
    echo "║  Proto:    TypeScript generated from echo.proto          ║"
    echo "║  WASM:     IndexedDB mailbox (rexie)                     ║"
    echo "║  Status:   Press Ctrl+C to stop                          ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo ""

    log_success "🎉 Everything is running!"
    echo ""
    log_info "Open http://localhost:3000 in your browser"
    log_info "Press Ctrl+C to stop all services"
    echo ""
    log_info "Quick test:"
    echo "  1. Click 'Connect to Server'"
    echo "  2. Type a message and click 'Send Echo'"
    echo "  3. Watch the real Actor-RTC call and IndexedDB storage!"
    echo ""
    log_info "View logs:"
    echo "  tail -f $SCRIPT_DIR/actrix.log  # Actrix signaling logs"
    echo "  tail -f $SCRIPT_DIR/server.log  # Actor-RTC server logs"
    echo "  tail -f $SCRIPT_DIR/client.log  # Web client logs"
    echo ""

    # Wait
    wait
}

main
