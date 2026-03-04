#!/bin/bash
# Start data-stream-peer-concurrent example (Python) - Bidirectional peer-to-peer streaming with concurrent clients
# Auto-starts actrix, server, and multiple clients to demonstrate concurrency

set -e
set -o pipefail

# Save initial working directory (allows script to be run from any directory)
INITIAL_PWD="$(pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

show_usage() {
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}📡 Data Stream Peer Concurrent Example (Python)${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "${GREEN}Usage:${NC}"
    echo "  $0 [OPTIONS]"
    echo ""
    echo -e "${GREEN}Options:${NC}"
    echo "  -c, --count <number>         Number of messages each client should receive (default: 5)"
    echo "  -n, --num-clients <n>        Number of concurrent clients to start (default: 3)"
    echo "  -a, --actrix-dir <path>      Path to actrix directory (default: auto-detect)"
    echo "  -h, --help                   Show this help message"
    echo ""
    echo -e "${GREEN}Examples:${NC}"
    echo "  $0                           # 3 clients, 5 messages each"
    echo "  $0 -c 10                     # 3 clients, 10 messages each"
    echo "  $0 -c 5 -n 5                 # 5 clients, 5 messages each"
    echo "  $0 -a /path/to/actrix        # Specify custom actrix directory"
    echo ""
    exit 0
}

MESSAGE_COUNT=5
NUM_CLIENTS=3
ACTRIX_DIR_OVERRIDE=""

while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            show_usage
            ;;
        -c|--count)
            MESSAGE_COUNT="$2"
            shift 2
            ;;
        -n|--num-clients)
            NUM_CLIENTS="$2"
            shift 2
            ;;
        -a|--actrix-dir)
            ACTRIX_DIR_OVERRIDE="$2"
            shift 2
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            show_usage
            exit 1
            ;;
    esac
done

if ! [[ "$MESSAGE_COUNT" =~ ^[0-9]+$ ]] || [ "$MESSAGE_COUNT" -lt 1 ]; then
    echo -e "${RED}Error: MESSAGE_COUNT must be a positive integer${NC}"
    exit 1
fi

if ! [[ "$NUM_CLIENTS" =~ ^[0-9]+$ ]] || [ "$NUM_CLIENTS" -lt 1 ]; then
    echo -e "${RED}Error: NUM_CLIENTS must be a positive integer${NC}"
    exit 1
fi

# Determine paths - ensure all paths are absolute (must be done before printing config)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
EXAMPLES_ROOT="$PROJECT_ROOT/actr-examples"

# Use override if provided, otherwise use default path
if [ -n "$ACTRIX_DIR_OVERRIDE" ]; then
    # Convert to absolute path
    if [[ "$ACTRIX_DIR_OVERRIDE" == /* ]]; then
        # Already absolute path
        ACTRIX_DIR="$ACTRIX_DIR_OVERRIDE"
    else
        # Relative path - resolve from initial working directory
        ACTRIX_DIR="$(cd "$INITIAL_PWD/$ACTRIX_DIR_OVERRIDE" 2>/dev/null && pwd || echo "$ACTRIX_DIR_OVERRIDE")"
    fi
    # Verify the resolved path exists
    if [ ! -d "$ACTRIX_DIR" ]; then
        echo -e "${YELLOW}⚠️  Warning: Specified actrix directory does not exist: $ACTRIX_DIR${NC}" >&2
    fi
else
    ACTRIX_DIR="$PROJECT_ROOT/actrix"
fi

ACTR_PYTHON_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROTO_DIR="$SCRIPT_DIR/proto"
GEN_DIR="$SCRIPT_DIR/generated"
VENV_DIR="$SCRIPT_DIR/.venv"
PYTHON="$VENV_DIR/bin/python"

# Verify critical directories exist
verify_paths() {
    local errors=0
    
    if [ ! -d "$PROJECT_ROOT" ]; then
        echo -e "${RED}❌ PROJECT_ROOT not found: $PROJECT_ROOT${NC}" >&2
        errors=$((errors + 1))
    fi
    
    if [ ! -d "$ACTRIX_DIR" ]; then
        echo -e "${RED}❌ ACTRIX_DIR not found: $ACTRIX_DIR${NC}" >&2
        if [ -n "$ACTRIX_DIR_OVERRIDE" ]; then
            echo -e "${RED}   (You specified: --actrix-dir $ACTRIX_DIR_OVERRIDE)${NC}" >&2
        fi
        errors=$((errors + 1))
    fi
    
    if [ ! -d "$ACTR_PYTHON_DIR" ]; then
        echo -e "${RED}❌ ACTR_PYTHON_DIR not found: $ACTR_PYTHON_DIR${NC}" >&2
        errors=$((errors + 1))
    fi
    
    if [ $errors -gt 0 ]; then
        echo -e "${RED}❌ Path verification failed. Please ensure script is in correct location.${NC}" >&2
        exit 1
    fi
}

# Verify paths early
verify_paths

# Print configuration (after paths are determined)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📡 Data Stream Peer Concurrent Example (Python)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo -e "${GREEN}Configuration:${NC}"
echo "   Message count per client: $MESSAGE_COUNT"
echo "   Number of concurrent clients: $NUM_CLIENTS"
if [ -n "$ACTRIX_DIR_OVERRIDE" ]; then
    echo "   Actrix directory: $ACTRIX_DIR (custom)"
else
    echo "   Actrix directory: $ACTRIX_DIR (auto-detected)"
fi
echo ""

# Create logs directory
LOG_DIR="$PROJECT_ROOT/logs"
mkdir -p "$LOG_DIR"

# Check Actr.example.toml files exist
echo ""
echo "🔍 Checking Actr.example.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

SERVER_ACTR_CONFIG="$SCRIPT_DIR/server/Actr.example.toml"
CLIENT_ACTR_CONFIG="$SCRIPT_DIR/client/Actr.example.toml"

if [ ! -f "$SERVER_ACTR_CONFIG" ]; then
    echo -e "${RED}❌ Actr.example.toml not found at $SERVER_ACTR_CONFIG${NC}" >&2
    exit 1
fi

if [ ! -f "$CLIENT_ACTR_CONFIG" ]; then
    echo -e "${RED}❌ Actr.example.toml not found at $CLIENT_ACTR_CONFIG${NC}" >&2
    exit 1
fi

echo -e "${GREEN}✅ Server Actr.example.toml found${NC}"
echo -e "${GREEN}✅ Client Actr.example.toml found${NC}"

# Determine actrix-config.example.toml path (use the one in Python example directory, fallback to examples root)
if [ -f "$SCRIPT_DIR/actrix-config.example.toml" ]; then
    ACTRIX_CONFIG="$SCRIPT_DIR/actrix-config.example.toml"
    echo -e "${GREEN}✅ Using actrix-config.example.toml from Python example directory${NC}"
elif [ -f "$EXAMPLES_ROOT/actrix-config.example.toml" ]; then
    ACTRIX_CONFIG="$EXAMPLES_ROOT/actrix-config.example.toml"
    echo -e "${GREEN}✅ Using actrix-config.example.toml from examples root${NC}"
else
    echo -e "${RED}❌ actrix-config.example.toml not found${NC}" >&2
    exit 1
fi

# Function to kill process on port
kill_port() {
    local port=$1
    local pid=$(lsof -ti:$port 2>/dev/null || true)
    if [ ! -z "$pid" ]; then
        echo "Killing process on port $port (PID: $pid)"
        kill -9 $pid 2>/dev/null || true
        sleep 0.5
    fi
}

# Cleanup function
cleanup() {
    echo ""
    echo "🧹 Cleaning up..."

    # Kill all clients
    for pid in "${CLIENT_PIDS[@]}"; do
        if [ ! -z "$pid" ]; then
            echo "Stopping client (PID: $pid)"
            kill $pid 2>/dev/null || true
        fi
    done

    # Kill server
    if [ ! -z "$SERVER_PID" ]; then
        echo "Stopping server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
    fi

    # Kill actrix
    if [ ! -z "$ACTRIX_PID" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    wait 2>/dev/null || true

    echo "✅ Cleanup complete"
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Step 0: Setup Python virtual environment
echo ""
echo "🐍 Step 0: Setting up Python environment..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ ! -d "$VENV_DIR" ]; then
    echo "Creating Python virtual environment..."
    python3 -m venv "$VENV_DIR"
fi

source "$VENV_DIR/bin/activate"
echo "Upgrading pip..."
python -m pip install -U pip >/dev/null 2>&1

echo "Installing Python dependencies..."
python -m pip install -r "$SCRIPT_DIR/requirements.txt" >/dev/null 2>&1

# Install mypy-protobuf for type stub generation (if not already installed)
if ! python -c "import mypy_protobuf" >/dev/null 2>&1; then
    echo "Installing mypy-protobuf for IDE type hints..."
    python -m pip install mypy-protobuf >/dev/null 2>&1
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✅ mypy-protobuf installed${NC}"
    else
        echo -e "${YELLOW}⚠️  Failed to install mypy-protobuf (type hints may not be available)${NC}"
    fi
fi

echo "Building and installing actr..."
export PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1
maturin develop --manifest-path "$ACTR_PYTHON_DIR/Cargo.toml" -q > "$LOG_DIR/maturin-build.log" 2>&1
if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ Python environment ready${NC}"
else
    echo -e "${RED}❌ Failed to build actr${NC}"
    cat "$LOG_DIR/maturin-build.log"
    exit 1
fi

# Step 1: Generate Python protobuf code
echo ""
echo "🔧 Step 1: Generating Python protobuf code..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! command -v protoc >/dev/null 2>&1; then
    echo -e "${RED}❌ protoc not found. Please install protoc first.${NC}"
    exit 1
fi

mkdir -p "$GEN_DIR"
touch "$GEN_DIR/__init__.py"
# Note: actr directory will be created by protoc, then renamed to actr_proto
# Remove old actr_proto directory if it exists
if [ -d "$GEN_DIR/actr_proto" ]; then
    rm -rf "$GEN_DIR/actr_proto"
fi

# Generate data_stream_peer protobuf
echo "Generating data_stream_peer protobuf..."
protoc -I "$PROTO_DIR" --python_out "$GEN_DIR" "$PROTO_DIR/data_stream_peer.proto"
if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ data_stream_peer protobuf generated${NC}"
else
    echo -e "${RED}❌ Failed to generate data_stream_peer protobuf${NC}"
    exit 1
fi

# Generate type stubs for IDE support (if mypy-protobuf is available)
if python -c "import mypy_protobuf" >/dev/null 2>&1; then
    echo "Generating data_stream_peer type stubs..."
    
    # Try to find protoc-gen-mypy in virtual environment first, then system PATH
    MYPY_PLUGIN=""
    if [ -f "$VENV_DIR/bin/protoc-gen-mypy" ]; then
        MYPY_PLUGIN="$VENV_DIR/bin/protoc-gen-mypy"
    elif command -v protoc-gen-mypy >/dev/null 2>&1; then
        MYPY_PLUGIN="protoc-gen-mypy"
    else
        # Try to find it in mypy_protobuf package
        MYPY_PLUGIN=$(python -c "import mypy_protobuf; import os; print(os.path.join(os.path.dirname(mypy_protobuf.__file__), 'protoc_gen_mypy.py'))" 2>/dev/null)
        if [ -z "$MYPY_PLUGIN" ] || [ ! -f "$MYPY_PLUGIN" ]; then
            MYPY_PLUGIN=""
        fi
    fi
    
    if [ -n "$MYPY_PLUGIN" ]; then
        if [ "$MYPY_PLUGIN" != "protoc-gen-mypy" ]; then
            protoc -I "$PROTO_DIR" \
                --plugin=protoc-gen-mypy="$MYPY_PLUGIN" \
                --mypy_out "$GEN_DIR" \
                "$PROTO_DIR/data_stream_peer.proto" 2>/dev/null
        else
            protoc -I "$PROTO_DIR" --mypy_out "$GEN_DIR" "$PROTO_DIR/data_stream_peer.proto" 2>/dev/null
        fi
        if [ $? -eq 0 ]; then
            echo -e "${GREEN}✅ data_stream_peer type stubs generated${NC}"
        fi
    fi
fi

# Generate actr protocol protobuf (for DataStream decode)
echo "Generating actr protocol protobuf..."
ACTR_PROTO_DIR="$PROJECT_ROOT/actr/crates/protocol/proto"
protoc -I "$ACTR_PROTO_DIR" --python_out "$GEN_DIR" \
  "$ACTR_PROTO_DIR/actr/options.proto" \
  "$ACTR_PROTO_DIR/actr.proto" \
  "$ACTR_PROTO_DIR/package.proto"
if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ actr protocol protobuf generated${NC}"
else
    echo -e "${RED}❌ Failed to generate actr protocol protobuf${NC}"
    exit 1
fi

# Generate type stubs for actr protocol (if mypy-protobuf is available)
if python -c "import mypy_protobuf" >/dev/null 2>&1; then
    echo "Generating actr protocol type stubs..."
    
    # Try to find protoc-gen-mypy in virtual environment first, then system PATH
    MYPY_PLUGIN=""
    if [ -f "$VENV_DIR/bin/protoc-gen-mypy" ]; then
        MYPY_PLUGIN="$VENV_DIR/bin/protoc-gen-mypy"
    elif command -v protoc-gen-mypy >/dev/null 2>&1; then
        MYPY_PLUGIN="protoc-gen-mypy"
    else
        # Try to find it in mypy_protobuf package
        MYPY_PLUGIN=$(python -c "import mypy_protobuf; import os; print(os.path.join(os.path.dirname(mypy_protobuf.__file__), 'protoc_gen_mypy.py'))" 2>/dev/null)
        if [ -z "$MYPY_PLUGIN" ] || [ ! -f "$MYPY_PLUGIN" ]; then
            MYPY_PLUGIN=""
        fi
    fi
    
    if [ -n "$MYPY_PLUGIN" ]; then
        if [ "$MYPY_PLUGIN" != "protoc-gen-mypy" ]; then
            protoc -I "$ACTR_PROTO_DIR" \
                --plugin=protoc-gen-mypy="$MYPY_PLUGIN" \
                --mypy_out "$GEN_DIR" \
                "$ACTR_PROTO_DIR/actr/options.proto" \
                "$ACTR_PROTO_DIR/actr.proto" \
                "$ACTR_PROTO_DIR/package.proto" 2>/dev/null
        else
            protoc -I "$ACTR_PROTO_DIR" --mypy_out "$GEN_DIR" \
              "$ACTR_PROTO_DIR/actr/options.proto" \
              "$ACTR_PROTO_DIR/actr.proto" \
              "$ACTR_PROTO_DIR/package.proto" 2>/dev/null
        fi
        if [ $? -eq 0 ]; then
            echo -e "${GREEN}✅ actr protocol type stubs generated${NC}"
        fi
    fi
fi

# Rename generated/actr/ to generated/actr_proto/ to avoid conflict with actr package
if [ -d "$GEN_DIR/actr" ]; then
    # Remove old actr_proto directory if it exists
    if [ -d "$GEN_DIR/actr_proto" ]; then
        rm -rf "$GEN_DIR/actr_proto"
    fi
    mv "$GEN_DIR/actr" "$GEN_DIR/actr_proto"
    echo -e "${GREEN}✅ Renamed actr/ to actr_proto/ to avoid naming conflict${NC}"
    
    # Note: Type stub files (.pyi) are included in the directory move above
fi

# Step 2: Build actrix (signaling server)
echo ""
echo "🏗️  Step 2: Building actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine if we need to build (check if ACTRIX_DIR is a build output directory)
NEED_BUILD=true
ACTRIX_SOURCE_DIR="$ACTRIX_DIR"
if [[ "$ACTRIX_DIR" == */target/debug ]] || [[ "$ACTRIX_DIR" == */target/release ]]; then
    # User specified a build output directory, extract source directory
    ACTRIX_SOURCE_DIR="${ACTRIX_DIR%/target/debug}"
    ACTRIX_SOURCE_DIR="${ACTRIX_SOURCE_DIR%/target/release}"
    # Check if binary already exists (user might have pre-built it)
    if [[ "$ACTRIX_DIR" == */target/debug ]] && [ -f "$ACTRIX_DIR/actrix" ]; then
        echo -e "${GREEN}✅ Using pre-built debug actrix binary${NC}"
        NEED_BUILD=false
    elif [[ "$ACTRIX_DIR" == */target/release ]] && [ -f "$ACTRIX_DIR/actrix" ]; then
        echo -e "${GREEN}✅ Using pre-built release actrix binary${NC}"
        NEED_BUILD=false
    fi
fi

if [ "$NEED_BUILD" = true ]; then
    # Verify source directory exists before cd
    if [ ! -d "$ACTRIX_SOURCE_DIR" ]; then
        echo -e "${RED}❌ Actrix source directory not found: $ACTRIX_SOURCE_DIR${NC}" >&2
        exit 1
    fi

    cd "$ACTRIX_SOURCE_DIR" || {
        echo -e "${RED}❌ Failed to change to actrix directory: $ACTRIX_SOURCE_DIR${NC}" >&2
        exit 1
    }

    # Determine build type based on ACTRIX_DIR
    if [[ "$ACTRIX_DIR" == */target/debug ]]; then
        BUILD_TYPE="debug"
        cargo build --bin actrix > "$LOG_DIR/actrix-build.log" 2>&1
    else
        BUILD_TYPE="release"
        cargo build --release --bin actrix > "$LOG_DIR/actrix-build.log" 2>&1
    fi
    
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✅ Actrix built successfully ($BUILD_TYPE)${NC}"
    else
        echo -e "${RED}❌ Actrix build failed${NC}"
        cat "$LOG_DIR/actrix-build.log"
        cd "$INITIAL_PWD" || true
        exit 1
    fi

    # Return to initial directory
    cd "$INITIAL_PWD" || true
else
    # Skip build, binary already exists
    BUILD_TYPE=""
fi

# Step 3: Start actrix (signaling server)
echo ""
echo "🚀 Step 3: Starting actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if actrix is already running
if lsof -ti:8081 >/dev/null 2>&1; then
    echo -e "${YELLOW}⚠️  Port 8081 already in use, attempting to kill existing process${NC}"
    kill_port 8081
fi

# Determine actrix binary path
# Check if ACTRIX_DIR already points to a build output directory (target/debug or target/release)
if [[ "$ACTRIX_DIR" == */target/debug ]] || [[ "$ACTRIX_DIR" == */target/release ]]; then
    # User specified a build output directory, use actrix directly from there
    ACTRIX_BIN="$ACTRIX_DIR/actrix"
else
    # User specified source directory, use target/release/actrix
    ACTRIX_BIN="$ACTRIX_DIR/target/release/actrix"
fi

# Verify actrix binary exists
if [ ! -f "$ACTRIX_BIN" ]; then
    echo -e "${RED}❌ Actrix binary not found: $ACTRIX_BIN${NC}" >&2
    echo -e "${RED}   Please ensure actrix was built successfully in Step 2${NC}" >&2
    # Suggest alternative paths if release not found
    if [[ "$ACTRIX_DIR" == */target/release ]] || [[ ! "$ACTRIX_DIR" == */target/debug ]]; then
        DEBUG_BIN="${ACTRIX_DIR%/target/release}/target/debug/actrix"
        if [ -f "$DEBUG_BIN" ]; then
            echo -e "${YELLOW}   Note: Debug binary found at: $DEBUG_BIN${NC}" >&2
        fi
    fi
    exit 1
fi

echo "Starting: $ACTRIX_BIN --config $ACTRIX_CONFIG"
$ACTRIX_BIN --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!
echo "$ACTRIX_PID" > "$LOG_DIR/actrix.pid"
echo -e "${GREEN}✅ Actrix started (PID: $ACTRIX_PID)${NC}"
echo "   Log: $LOG_DIR/actrix.log"

# Wait for actrix to be ready
echo "⏳ Waiting for actrix to start..."
for i in {1..20}; do
    if lsof -Pi :8081 -sTCP:LISTEN -t >/dev/null 2>&1 && lsof -Pi :50055 -sTCP:LISTEN -t >/dev/null 2>&1; then
        echo -e "${GREEN}✅ Actrix is listening on 8081 and 50055${NC}"
        break
    fi
    if [ "$i" -eq 20 ]; then
        echo -e "${RED}❌ Actrix did not become ready in time. Check logs:${NC}"
        tail -20 "$LOG_DIR/actrix.log"
        exit 1
    fi
    sleep 1
done

# Check if actrix is still running
if ! kill -0 $ACTRIX_PID 2>/dev/null; then
    echo -e "${RED}❌ Actrix failed to start. Check logs:${NC}"
    tail -20 "$LOG_DIR/actrix.log"
    exit 1
fi

# Step 4: Setup realms
echo ""
echo "🔑 Step 4: Setting up realms..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Temporarily disable set -e for this section to allow graceful failure handling
set +e

if [ -d "$EXAMPLES_ROOT" ]; then
    cd "$EXAMPLES_ROOT" || {
        echo -e "${YELLOW}⚠️  Failed to change to $EXAMPLES_ROOT (will continue)${NC}"
        set -e
        cd "$INITIAL_PWD" || true
    }
    
    cargo build -p realm-setup > "$LOG_DIR/realm-setup-build.log" 2>&1
    BUILD_STATUS=$?

    if [ $BUILD_STATUS -ne 0 ]; then
        echo -e "${YELLOW}⚠️  Failed to build realm-setup (will continue)${NC}"
        echo "   Check log: $LOG_DIR/realm-setup-build.log"
    else
        echo "Running realm-setup..."
        cargo run -p realm-setup -- \
            --actrix-config "$ACTRIX_CONFIG" \
            --actr-toml "$SERVER_ACTR_CONFIG" \
            --actr-toml "$CLIENT_ACTR_CONFIG" \
            > "$LOG_DIR/realm-setup.log" 2>&1
        
        if [ $? -eq 0 ]; then
            echo -e "${GREEN}✅ Realms setup completed${NC}"
        else
            echo -e "${YELLOW}⚠️  Realm setup failed (will continue). See $LOG_DIR/realm-setup.log${NC}"
        fi
    fi
    
    # Return to initial directory
    cd "$INITIAL_PWD" || true
else
    echo -e "${YELLOW}⚠️  EXAMPLES_ROOT directory not found, skipping realm-setup${NC}"
fi

# Re-enable set -e
set -e

# Step 5: Start server
echo ""
echo "🚀 Step 5: Starting Python server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Verify directory exists before cd
if [ ! -d "$SCRIPT_DIR/server" ]; then
    echo -e "${RED}❌ Server directory not found: $SCRIPT_DIR/server${NC}" >&2
    exit 1
fi

cd "$SCRIPT_DIR/server" || {
    echo -e "${RED}❌ Failed to change to server directory: $SCRIPT_DIR/server${NC}" >&2
    exit 1
}

$PYTHON server.py --actr-toml Actr.example.toml > "$LOG_DIR/data-stream-peer-concurrent-py-server.log" 2>&1 &
SERVER_PID=$!

# Return to initial directory (process is in background)
cd "$INITIAL_PWD" || true
echo -e "${GREEN}✅ Server started (PID: $SERVER_PID)${NC}"
echo "   Log: $LOG_DIR/data-stream-peer-concurrent-py-server.log"

# Wait for server to initialize and register with signaling server
echo "⏳ Waiting for server to initialize and register..."
for i in {1..20}; do
    if grep -q "Python Server started" "$LOG_DIR/data-stream-peer-concurrent-py-server.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Server started successfully${NC}"
        sleep 2  # Give actrix time to register the server
        break
    fi
    if [ "$i" -eq 20 ]; then
        echo -e "${RED}❌ Server did not start in time. Check logs:${NC}"
        tail -20 "$LOG_DIR/data-stream-peer-concurrent-py-server.log"
        exit 1
    fi
    sleep 1
done

# Check if server is still running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo -e "${RED}❌ Server failed to start. Check logs:${NC}"
    tail -20 "$LOG_DIR/data-stream-peer-concurrent-py-server.log"
    exit 1
fi

# Step 6: Start multiple clients to demonstrate concurrency
echo ""
echo "🚀 Step 6: Starting multiple clients (concurrency test)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Array to track client PIDs
declare -a CLIENT_PIDS

# Concurrently start all clients
echo "🚀 Starting $NUM_CLIENTS clients concurrently..."

# Verify directory exists before cd
if [ ! -d "$SCRIPT_DIR/client" ]; then
    echo -e "${RED}❌ Client directory not found: $SCRIPT_DIR/client${NC}" >&2
    exit 1
fi

cd "$SCRIPT_DIR/client" || {
    echo -e "${RED}❌ Failed to change to client directory: $SCRIPT_DIR/client${NC}" >&2
    exit 1
}

for i in $(seq 1 $NUM_CLIENTS); do
    CLIENT_ID="client-$i"
    $PYTHON client.py --actr-toml Actr.example.toml "$CLIENT_ID" "$MESSAGE_COUNT" > "$LOG_DIR/data-stream-peer-concurrent-py-client-$i.log" 2>&1 &
    CLIENT_PID=$!
    CLIENT_PIDS+=($CLIENT_PID)
    echo -e "${GREEN}✅ Client #$i started (PID: $CLIENT_PID, ID: $CLIENT_ID)${NC}"
    echo "   Log: $LOG_DIR/data-stream-peer-concurrent-py-client-$i.log"
    sleep 1
done

# Return to initial directory
cd "$INITIAL_PWD" || true

echo ""
echo "⏳ All clients started, waiting for connections to establish..."
sleep 2

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "${GREEN}✅ All processes started successfully${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "📊 Process Status:"
echo "   Actrix:  PID $ACTRIX_PID (log: logs/actrix.log)"
echo "   Server:  PID $SERVER_PID (log: logs/data-stream-peer-concurrent-py-server.log)"
for i in $(seq 1 $NUM_CLIENTS); do
    echo "   Client #$i: PID ${CLIENT_PIDS[$((i-1))]} (log: logs/data-stream-peer-concurrent-py-client-$i.log)"
done
echo ""
echo "💡 To view logs in real-time:"
echo "   tail -f logs/data-stream-peer-concurrent-py-server.log"
echo "   tail -f logs/data-stream-peer-concurrent-py-client-1.log"
echo ""
echo "Press Ctrl+C to stop all processes"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait for all clients to complete
wait "${CLIENT_PIDS[@]}"

echo ""
echo "🎉 All clients completed their streaming!"
echo "📊 Check logs for detailed streaming statistics"
echo ""
echo "Press Ctrl+C to stop server and actrix, or they will continue running..."

# Keep server and actrix running
wait

