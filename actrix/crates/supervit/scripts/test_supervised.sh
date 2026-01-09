#!/bin/bash
#
# Test script for SupervisedService gRPC API using grpcurl
#
# Prerequisites:
# - grpcurl installed: brew install grpcurl (macOS) or go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest
# - Server running: cargo run -p supervit --example supervisord_server
#
# Usage:
#   ./scripts/test_supervised.sh [action] [options]
#
# Actions:
#   list          - List available gRPC services
#   describe      - Describe SupervisedService
#   node_info     - Get node information
#   list_realms   - List all realms
#   get_realm     - Get realm by ID (requires --realm-id)
#   create_realm  - Create a new realm (requires --realm-id)
#   shutdown      - Shutdown the node
#
# Examples:
#   ./scripts/test_supervised.sh list
#   ./scripts/test_supervised.sh node_info
#   ./scripts/test_supervised.sh get_realm --realm-id test-realm-01
#   ./scripts/test_supervised.sh create_realm --realm-id new-realm

set -e

# Configuration
SERVER_ADDR="${SERVER_ADDR:-127.0.0.1:50055}"
PROTO_PATH="${PROTO_PATH:-crates/actrix-proto/proto}"
PROTO_FILES="-proto supervised.proto -proto common.proto"
# Try config.toml first (for actrix), then fallback to config.test.toml
CONFIG_FILE="${CONFIG_FILE:-$([ -f config.toml ] && echo config.toml || echo config.test.toml)}"

if [[ "${USE_CONFIG:-false}" == "true" ]]; then
    # Try to read node_id and shared_secret from config file if it exists
    if [[ -f "${CONFIG_FILE}" ]]; then
        # Extract node_id from [supervisor.client] section (supports both quoted and unquoted values)
        if [[ -z "${NODE_ID}" ]]; then
            NODE_ID=$(awk '/\[supervisor.client\]/{flag=1; next} /^\[/{flag=0} flag && /^node_id/ {gsub(/[" ]/, "", $3); print $3; exit}' "${CONFIG_FILE}" 2>/dev/null)
        fi
        # Extract shared_secret from [supervisor.client] section
        if [[ -z "${SHARED_SECRET}" ]]; then
            SHARED_SECRET=$(awk '/\[supervisor.client\]/{flag=1; next} /^\[/{flag=0} flag && /^shared_secret/ {gsub(/[" ]/, "", $3); print $3; exit}' "${CONFIG_FILE}" 2>/dev/null)
        fi
    fi
fi

# Fallback to defaults if not set (align with supervisord_server example)
NODE_ID="${NODE_ID:-example-node-01}"
SHARED_SECRET="${SHARED_SECRET:-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

success() {
    echo -e "${GREEN}[OK]${NC} $*"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

error() {
    echo -e "${RED}[ERROR]${NC} $*" >&2
}

# Check if grpcurl is installed
check_grpcurl() {
    if ! command -v grpcurl &> /dev/null; then
        error "grpcurl is not installed"
        echo "Install with:"
        echo "  macOS: brew install grpcurl"
        echo "  Linux: go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest"
        exit 1
    fi
}

# Check if proto files exist
check_proto_files() {
    if [[ ! -f "${PROTO_PATH}/supervised.proto" ]]; then
        error "Proto files not found at ${PROTO_PATH}"
        echo "Make sure you run this script from the project root directory"
        exit 1
    fi
}

# Generate credential using the Rust helper
generate_credential() {
    local action="$1"
    local subject="$2"
    
    local cmd="cargo run -p supervit --bin gen_credential --quiet -- --action ${action} --output json --node-id ${NODE_ID} --shared-secret ${SHARED_SECRET}"
    if [[ -n "$subject" ]]; then
        cmd="$cmd --subject ${subject}"
    fi
    
    # Run the command and capture the JSON output
    eval "$cmd" 2>/dev/null
}

# Parse command line arguments
parse_args() {
    ACTION=""
    REALM_ID=""
    
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --realm-id)
                REALM_ID="$2"
                shift 2
                ;;
            --server)
                SERVER_ADDR="$2"
                shift 2
                ;;
            --help|-h)
                show_help
                exit 0
                ;;
            *)
                if [[ -z "$ACTION" ]]; then
                    ACTION="$1"
                fi
                shift
                ;;
        esac
    done
}

show_help() {
    cat << 'EOF'
SupervisedService gRPC Test Script

Usage: ./scripts/test_supervised.sh [action] [options]

Actions:
  list          List available gRPC services (no auth required)
  describe      Describe SupervisedService methods (no auth required)
  node_info     Get node information
  list_realms   List all realms
  get_realm     Get realm by ID (requires --realm-id)
  create_realm  Create a new realm (requires --realm-id)
  shutdown      Shutdown the node gracefully

Options:
  --realm-id    Realm ID for realm operations
  --server      gRPC server address (default: localhost:50055)
  --help, -h    Show this help message

Environment Variables:
  SERVER_ADDR      Override server address
  PROTO_PATH       Override proto file path (default: crates/actrix-proto/proto)
  CONFIG_FILE      Config file path to read node_id and shared_secret (default: config.test.toml)
  NODE_ID          Override node_id (default: read from config file or test-node-01)
  SHARED_SECRET    Override shared_secret (default: read from config file or default test value)

Examples:
  # List services (no auth)
  ./scripts/test_supervised.sh list

  # Get node info
  ./scripts/test_supervised.sh node_info

  # List all realms
  ./scripts/test_supervised.sh list_realms

  # Get specific realm
  ./scripts/test_supervised.sh get_realm --realm-id my-realm

  # Create realm
  ./scripts/test_supervised.sh create_realm --realm-id new-realm

Prerequisites:
  1. Start the server: cargo run -p supervit --example supervisord_server
  2. Install grpcurl: brew install grpcurl
EOF
}

# Action: List services
action_list() {
    info "Listing gRPC services..."
    grpcurl -plaintext -import-path "${PROTO_PATH}" ${PROTO_FILES} "${SERVER_ADDR}" list
}

# Action: Describe service
action_describe() {
    info "Describing SupervisedService..."
    grpcurl -plaintext -import-path "${PROTO_PATH}" ${PROTO_FILES} "${SERVER_ADDR}" describe supervisor.v1.SupervisedService
}

# Action: Get node info
action_node_info() {
    info "Getting node info..."
    
    local cred
    cred=$(generate_credential "node_info")
    if [[ -z "$cred" ]]; then
        error "Failed to generate credential"
        exit 1
    fi
    
    local request
    request=$(cat << EOF
{
  "credential": ${cred}
}
EOF
)
    
    grpcurl -plaintext -import-path "${PROTO_PATH}" ${PROTO_FILES} \
        -d "${request}" \
        "${SERVER_ADDR}" supervisor.v1.SupervisedService/GetNodeInfo
}

# Action: List realms
action_list_realms() {
    info "Listing realms..."

    local cred
    cred=$(generate_credential "list_realms")
    if [[ -z "$cred" ]]; then
        error "Failed to generate credential"
        exit 1
    fi

    local request
    request=$(cat << EOF
{
  "credential": ${cred}
}
EOF
)

    grpcurl -plaintext -import-path "${PROTO_PATH}" ${PROTO_FILES} \
        -d "${request}" \
        "${SERVER_ADDR}" supervisor.v1.SupervisedService/ListRealms
}

# Action: Get realm
action_get_realm() {
    if [[ -z "$REALM_ID" ]]; then
        error "Realm ID required. Use --realm-id <id>"
        exit 1
    fi

    info "Getting realm: ${REALM_ID}"

    local cred
    cred=$(generate_credential "get_realm" "${REALM_ID}")
    if [[ -z "$cred" ]]; then
        error "Failed to generate credential"
        exit 1
    fi

    local request
    request=$(cat << EOF
{
  "realm_id": $(echo "${REALM_ID}" | grep -E '^[0-9]+$' >/dev/null && echo "${REALM_ID}" || echo "1"),
  "credential": ${cred}
}
EOF
)

    grpcurl -plaintext -import-path "${PROTO_PATH}" ${PROTO_FILES} \
        -d "${request}" \
        "${SERVER_ADDR}" supervisor.v1.SupervisedService/GetRealm
}

# Action: Create realm
action_create_realm() {
    if [[ -z "$REALM_ID" ]]; then
        error "Realm ID required. Use --realm-id <id>"
        exit 1
    fi

    info "Creating realm: ${REALM_ID}"

    local cred
    cred=$(generate_credential "create_realm" "${REALM_ID}")
    if [[ -z "$cred" ]]; then
        error "Failed to generate credential"
        exit 1
    fi

    # Generate a test public key (33 bytes, base64 encoded)
    # This is a compressed secp256k1 public key format (0x02 or 0x03 prefix + 32 bytes)
    local test_public_key="AhY2dJI5sP3r8wqO0K5rT1nL2mX4yU6vB8cA9dEfGhIj"

    local request
    request=$(cat << EOF
{
  "realm_id": $(echo "${REALM_ID}" | grep -E '^[0-9]+$' >/dev/null && echo "${REALM_ID}" || echo "1"),
  "name": "Test Realm ${REALM_ID}",
  "public_key": "${test_public_key}",
  "enabled": true,
  "key_id": $(date +%s),
  "use_servers": [1, 2, 3],
  "version": 1,
  "credential": ${cred}
}
EOF
)

    grpcurl -plaintext -import-path "${PROTO_PATH}" ${PROTO_FILES} \
        -d "${request}" \
        "${SERVER_ADDR}" supervisor.v1.SupervisedService/CreateRealm
}

# Action: Shutdown
action_shutdown() {
    warn "Requesting node shutdown..."
    
    local cred
    cred=$(generate_credential "shutdown")
    if [[ -z "$cred" ]]; then
        error "Failed to generate credential"
        exit 1
    fi
    
    local request
    request=$(cat << EOF
{
  "graceful": true,
  "timeout_secs": 30,
  "reason": "Test shutdown via grpcurl",
  "credential": ${cred}
}
EOF
)
    
    grpcurl -plaintext -import-path "${PROTO_PATH}" ${PROTO_FILES} \
        -d "${request}" \
        "${SERVER_ADDR}" supervisor.v1.SupervisedService/Shutdown
}

# Main
main() {
    parse_args "$@"
    
    # Default action
    if [[ -z "$ACTION" ]]; then
        show_help
        exit 0
    fi
    
    check_grpcurl
    check_proto_files
    
    case "$ACTION" in
        list)
            action_list
            ;;
        describe)
            action_describe
            ;;
        node_info|node-info|nodeinfo)
            action_node_info
            ;;
        list_realms|list-realms|listrealms)
            action_list_realms
            ;;
        get_realm|get-realm|getrealm)
            action_get_realm
            ;;
        create_realm|create-realm|createrealms)
            action_create_realm
            ;;
        shutdown)
            action_shutdown
            ;;
        *)
            error "Unknown action: ${ACTION}"
            echo ""
            show_help
            exit 1
            ;;
    esac
}

main "$@"
