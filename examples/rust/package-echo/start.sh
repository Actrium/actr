#!/bin/bash
# Test script for package-echo example — echo-actr package loaded from local build
#
# Demonstrates the full package-driven execution flow:
#   1. Build the local echo-actr wasm package and reuse its public key
#   2. Verify the signed .actr archive
#   3. Host server loads the package and picks the workload from package target
#   4. Client discovers the echo service, sends messages, verifies responses
#
# Usage:
#   ./start.sh              # Use default message "TestMsg"
#   ./start.sh "Hello"      # Send custom message

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Testing package-echo (local echo-actr package loader)"
echo "    Using Actrix as signaling server"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Paths ────────────────────────────────────────────────────────────────

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Repo root is 2 levels up from WORKSPACE_ROOT (examples/rust)
ACTR_REPO_DIR="$(cd "$WORKSPACE_ROOT/../.." && pwd)"
# Actrium root is one level above the repo root
ACTRIUM_DIR="$(cd "$ACTR_REPO_DIR/.." && pwd)"
ACTRIX_DIR="$ACTRIUM_DIR/actrix"
ACTR_CLI_MANIFEST="$ACTR_REPO_DIR/cli/Cargo.toml"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
PACKAGE_ECHO_DIR="$WORKSPACE_ROOT/package-echo"
CLIENT_DIR="$PACKAGE_ECHO_DIR/client"
CLIENT_GUEST_DIR="$PACKAGE_ECHO_DIR/client-guest"
ECHO_ACTR_DIR="$WORKSPACE_ROOT/echo-actr"

# Ensure ~/.cargo/bin is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

# ── Check and install jq if needed ───────────────────────────────────────
if ! command -v jq >/dev/null 2>&1; then
    echo ""
    echo -e "${YELLOW}⚠️  jq not found, attempting to install...${NC}"

    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS - use Homebrew
        if command -v brew >/dev/null 2>&1; then
            echo "Installing jq via Homebrew..."
            brew install jq
        else
            echo -e "${RED}❌ Homebrew not found. Please install jq manually:${NC}"
            echo "   brew install jq"
            echo "   or visit: https://jqlang.github.io/jq/download/"
            exit 1
        fi
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        # Linux - try common package managers
        if command -v apt-get >/dev/null 2>&1; then
            echo "Installing jq via apt-get..."
            sudo apt-get update && sudo apt-get install -y jq
        elif command -v yum >/dev/null 2>&1; then
            echo "Installing jq via yum..."
            sudo yum install -y jq
        elif command -v dnf >/dev/null 2>&1; then
            echo "Installing jq via dnf..."
            sudo dnf install -y jq
        else
            echo -e "${RED}❌ No supported package manager found. Please install jq manually.${NC}"
            exit 1
        fi
    else
        echo -e "${RED}❌ Unsupported OS: $OSTYPE. Please install jq manually.${NC}"
        exit 1
    fi

    # Verify installation
    if command -v jq >/dev/null 2>&1; then
        echo -e "${GREEN}✅ jq installed successfully: $(jq --version)${NC}"
    else
        echo -e "${RED}❌ jq installation failed${NC}"
        exit 1
    fi
else
    echo -e "${GREEN}✅ jq found: $(jq --version)${NC}"
fi

cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required helper scripts
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# ── Clean stale database and config files ────────────────────────────────
# Remove DB files from previous runs so actrix starts with fresh keys.
# Without this, expired signing keys cause "Invalid credential format" errors.
echo ""
echo "🗑️  Cleaning stale database files..."
rm -rf "$WORKSPACE_ROOT/database"
# Remove runtime config files to ensure they're freshly copied from Actr.example.toml,
# and manifest files from Actr.example.toml
rm -f "$CLIENT_DIR/actr.toml" "$CLIENT_GUEST_DIR/actr.toml"
echo -e "${GREEN}✅ Stale database cleaned${NC}"

# Ensure manifest.toml and actr.toml files exist
echo ""
echo "🔍 Checking config files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$CLIENT_DIR"
ensure_actr_toml "$CLIENT_GUEST_DIR"
# actr.toml = runtime config (from Actr.example.toml)
echo -e "${GREEN}✅ Synchronized actr.toml from Actr.example.toml${NC}"

# Ensure actrix-config.toml exists
ensure_actrix_config "$WORKSPACE_ROOT"

# ── Cleanup ──────────────────────────────────────────────────────────────

ACTRIX_PID=""
SERVER_PID=""
CLIENT_PID=""

cleanup() {
    echo ""
    echo "🧹 Cleaning up..."

    if [ -n "$ACTRIX_PID" ]; then
        echo "Stopping actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    if [ -n "$SERVER_PID" ]; then
        echo "Stopping package-echo-server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
    fi

    if [ -n "$CLIENT_PID" ]; then
        echo "Stopping package-echo-client (PID: $CLIENT_PID)"
        kill $CLIENT_PID 2>/dev/null || true
    fi

    wait 2>/dev/null || true
    echo "✅ Cleanup complete"
}

trap cleanup EXIT INT TERM

# ── Step 0: Compile echo-actr WASM ──────────────────────────────────────

echo ""
echo -e "${BLUE}📦 Step 0: Compiling echo-actr WASM...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

echo_actr_version() {
    awk '
        /^\[package\]/ { in_package = 1; next }
        /^\[/ && in_package { exit }
        in_package && $1 == "version" {
            gsub(/"/, "", $3)
            print $3
            exit
        }
    ' "$ECHO_ACTR_DIR/Cargo.toml"
}

ECHO_ACTR_VERSION="${ECHO_ACTR_VERSION:-$(echo_actr_version)}"
ECHO_ACTR_TARGET="wasm32-unknown-unknown"
SIGNING_KEY="$ECHO_ACTR_DIR/packaging/keys/dev-signing-key.json"
PUBLIC_KEY_PATH="$ECHO_ACTR_DIR/public-key.json"

if [ ! -d "$ECHO_ACTR_DIR" ] && [ -z "${ACTR_PACKAGE_PATH:-}" ]; then
    echo -e "${RED}❌ echo-actr repository not found: $ECHO_ACTR_DIR${NC}"
    exit 1
fi

echo "echo-actr dir: $ECHO_ACTR_DIR"
echo "Version:       $ECHO_ACTR_VERSION"
echo "Target:        $ECHO_ACTR_TARGET"

# Patch client actr.toml ACL to reference the correct EchoService version
perl -0pi -e "s/type = \"actrium:EchoService:[^\"]+\"/type = \"actrium:EchoService:${ECHO_ACTR_VERSION}\"/g" \
    "$CLIENT_DIR/actr.toml" "$CLIENT_GUEST_DIR/actr.toml" 2>/dev/null || true

# 1. Compile to WASM
rustup target add "$ECHO_ACTR_TARGET" >/dev/null
cargo build --manifest-path "$ECHO_ACTR_DIR/Cargo.toml" \
    --lib --release --target "$ECHO_ACTR_TARGET" 2>&1 | tail -5

# Resolve WASM output path: workspace may use a shared target directory
RAW_WASM=""
for _candidate in \
    "$ECHO_ACTR_DIR/target/${ECHO_ACTR_TARGET}/release/echo_guest.wasm" \
    "$(dirname "$ECHO_ACTR_DIR")/target/${ECHO_ACTR_TARGET}/release/echo_guest.wasm" \
    "$ACTR_REPO_DIR/target/examples/${ECHO_ACTR_TARGET}/release/echo_guest.wasm"; do
    if [ -f "$_candidate" ]; then
        RAW_WASM="$_candidate"
        break
    fi
done
if [ ! -f "$RAW_WASM" ]; then
    echo -e "${RED}❌ WASM compilation failed${NC}"
    exit 1
fi
echo -e "${GREEN}✅ WASM compiled: $(du -h "$RAW_WASM" | cut -f1)${NC}"

# 2. wasm-opt --asyncify
WASM_OPT_CMD="${WASM_OPT:-wasm-opt}"
if ! command -v "$WASM_OPT_CMD" >/dev/null 2>&1; then
    echo -e "${YELLOW}⚠️  wasm-opt not found, installing via cargo...${NC}"
    cargo install wasm-opt 2>&1 | tail -3
fi
mkdir -p "$ECHO_ACTR_DIR/dist"
OPTIMIZED_WASM="$ECHO_ACTR_DIR/dist/echo-actr-${ECHO_ACTR_VERSION}-${ECHO_ACTR_TARGET}.wasm"
"$WASM_OPT_CMD" --asyncify -O "$RAW_WASM" -o "$OPTIMIZED_WASM"
echo -e "${GREEN}✅ wasm-opt done: $(du -h "$OPTIMIZED_WASM" | cut -f1)${NC}"

# ── Step 0.5: Pack .actr with actr pkg build ──────────────────────────────

echo ""
echo -e "${BLUE}📦 Step 0.5: Packing signed .actr package via actr pkg build...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_PACKAGE="$ECHO_ACTR_DIR/dist/actrium-EchoService-${ECHO_ACTR_VERSION}-${ECHO_ACTR_TARGET}.actr"
BUILD_CONFIG="$ECHO_ACTR_DIR/dist/build-config.toml"

# Generate a temporary build manifest (manifest.toml format: exports under [package])
cat > "$BUILD_CONFIG" << TOML
[package]
manufacturer = "actrium"
name = "EchoService"
version = "$ECHO_ACTR_VERSION"
description = "Signed Echo guest actor"
license = "Apache-2.0"
exports = ["$ECHO_ACTR_DIR/proto/echo.proto"]
TOML

cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg build \
    --binary "$OPTIMIZED_WASM" \
    --config "$BUILD_CONFIG" \
    --key "$SIGNING_KEY" \
    --target "$ECHO_ACTR_TARGET" \
    --output "$ACTR_PACKAGE"

if [ ! -f "$ACTR_PACKAGE" ]; then
    echo -e "${RED}❌ actr pkg build failed: $ACTR_PACKAGE not found${NC}"
    exit 1
fi
echo -e "${GREEN}✅ .actr package built: $(du -h "$ACTR_PACKAGE" | cut -f1)${NC}"

# Verify the package signature
cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg verify \
    --pubkey "$PUBLIC_KEY_PATH" \
    --package "$ACTR_PACKAGE" >/dev/null
echo -e "${GREEN}✅ Package signature verified${NC}"

# ── Step 0.5: Build client-guest cdylib package ──────────────────────────

echo ""
echo -e "${BLUE}📦 Building client-guest cdylib package...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

CLIENT_GUEST_VERSION="0.1.0"
CLIENT_GUEST_TARGET="$(rustc -vV | awk '/host:/ {print $2}')"
CLIENT_GUEST_DIST_DIR="$CLIENT_GUEST_DIR/dist"
mkdir -p "$CLIENT_GUEST_DIST_DIR"

# Build the cdylib (runs in workspace root, so target is at workspace level)
# Use workspace-level target dir from .cargo/config.toml (target-dir = "../../target/examples")
WORKSPACE_TARGET_DIR="$ACTR_REPO_DIR/target/examples"
if ! cargo build --manifest-path "$CLIENT_GUEST_DIR/Cargo.toml" 2>&1; then
    echo -e "${RED}❌ Failed to build client-guest cdylib${NC}"
    exit 1
fi

# Locate the built dylib/so (workspace target dir)
if [ "$(uname)" = "Darwin" ]; then
    CLIENT_GUEST_BINARY="$WORKSPACE_TARGET_DIR/debug/libpackage_echo_client_guest.dylib"
elif [ "$(uname)" = "Linux" ]; then
    CLIENT_GUEST_BINARY="$WORKSPACE_TARGET_DIR/debug/libpackage_echo_client_guest.so"
else
    CLIENT_GUEST_BINARY="$WORKSPACE_TARGET_DIR/debug/package_echo_client_guest.dll"
fi

if [ ! -f "$CLIENT_GUEST_BINARY" ]; then
    echo -e "${RED}❌ client-guest binary not found: $CLIENT_GUEST_BINARY${NC}"
    exit 1
fi

CLIENT_GUEST_DEV_KEY="$PACKAGE_ECHO_DIR/dev-key.json"
CLIENT_GUEST_PACKAGE_NAME="acme-package-echo-client-guest-${CLIENT_GUEST_VERSION}-cdylib.actr"
CLIENT_GUEST_PACKAGE="$CLIENT_GUEST_DIST_DIR/$CLIENT_GUEST_PACKAGE_NAME"
CLIENT_GUEST_PUBLIC_KEY="$CLIENT_GUEST_DIST_DIR/public-key.json"

# Package the client-guest binary (uses manifest.toml = Actr.example.toml)
cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg build \
    --binary "$CLIENT_GUEST_BINARY" \
    --config "$CLIENT_GUEST_DIR/manifest.toml" \
    --key "$CLIENT_GUEST_DEV_KEY" \
    --target "$CLIENT_GUEST_TARGET" \
    --output "$CLIENT_GUEST_PACKAGE"

if [ ! -f "$CLIENT_GUEST_PACKAGE" ]; then
    echo -e "${RED}❌ client-guest package build failed: $CLIENT_GUEST_PACKAGE not found${NC}"
    exit 1
fi

# Extract public key from dev-key.json for client-guest
jq '{public_key: .public_key}' "$CLIENT_GUEST_DEV_KEY" > "$CLIENT_GUEST_PUBLIC_KEY"

echo -e "${GREEN}✅ client-guest package ready: $(du -h "$CLIENT_GUEST_PACKAGE" | cut -f1)${NC}"

# Verify client-guest package
cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg verify \
    --pubkey "$CLIENT_GUEST_PUBLIC_KEY" \
    --package "$CLIENT_GUEST_PACKAGE" >/dev/null

echo -e "${GREEN}✅ client-guest package verified${NC}"

# ── Step 1: Ensure actrix is available ──────────────────────────────────

echo ""
echo "📦 Checking actrix availability..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTRIX_CMD=""
if [ -x "$ACTRIX_DIR/target/debug/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
    echo -e "${GREEN}✅ Actrix found: $ACTRIX_CMD${NC}"
elif [ -x "$ACTRIX_DIR/target/release/actrix" ]; then
    ACTRIX_CMD="$ACTRIX_DIR/target/release/actrix"
    echo -e "${GREEN}✅ Actrix found: $ACTRIX_CMD${NC}"
elif command -v actrix > /dev/null 2>&1; then
    ACTRIX_CMD="actrix"
    echo -e "${YELLOW}⚠️  Falling back to actrix from PATH: $(which actrix)${NC}"
else
    echo -e "${YELLOW}⚠️  Actrix not found in PATH or build directory. Attempting build...${NC}"
    if [ -d "$ACTRIX_DIR" ]; then
        cd "$ACTRIX_DIR"
        cargo build 2>&1 | tail -5
        if [ -x "$ACTRIX_DIR/target/debug/actrix" ]; then
            ACTRIX_CMD="$ACTRIX_DIR/target/debug/actrix"
        fi
        cd "$WORKSPACE_ROOT"
    fi

    if [ -z "$ACTRIX_CMD" ]; then
        echo -e "${RED}❌ Actrix not available. Install it first or build from $ACTRIX_DIR${NC}"
        exit 1
    fi
fi

# ── Step 2: Start actrix ────────────────────────────────────────────────

echo ""
echo "🚀 Starting actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Ensure we do not talk to a stale actrix from another working directory.
if lsof -tiTCP:8081 -sTCP:LISTEN >/dev/null 2>&1; then
    echo "Removing stale listener on 8081..."
    kill $(lsof -tiTCP:8081 -sTCP:LISTEN) 2>/dev/null || true
    sleep 1
fi
if lsof -tiUDP:3478 >/dev/null 2>&1; then
    echo "Removing stale listener on 3478/udp..."
    kill $(lsof -tiUDP:3478) 2>/dev/null || true
    sleep 1
fi

$ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "Actrix started (PID: $ACTRIX_PID)"
echo "Waiting for actrix to be ready..."

actrix_http_ready() {
    lsof -nP -iTCP:8081 -sTCP:LISTEN > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null
}

actrix_ice_ready() {
    lsof -nP -iUDP:3478 > /dev/null 2>&1
}

# Wait for HTTP first (hard requirement)
MAX_WAIT=20
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi

    if actrix_http_ready; then
        echo -e "${GREEN}✅ Actrix HTTP is ready on port 8081/tcp${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix not listening on 8081/tcp after ${MAX_WAIT} seconds${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# Wait for ICE/TURN (soft check — give it up to 5 more seconds)
ICE_COUNTER=0
while [ $ICE_COUNTER -lt 5 ]; do
    if actrix_ice_ready 2>/dev/null; then
        echo -e "${GREEN}✅ Actrix ICE/TURN ready on 3478/udp${NC}"
        break
    fi
    sleep 1 || true
    ICE_COUNTER=$((ICE_COUNTER + 1))
done
if [ $ICE_COUNTER -eq 5 ]; then
    echo -e "${YELLOW}⚠️  3478/udp not detected (may be using host ICE); continuing...${NC}" || true
fi

# ── Step 2.5: Setup realms ──────────────────────────────────────────────

echo ""
echo "🔑 Setting up realms in actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Poll until the AIS signing key is ready.
# Strategy: call /ais/rotate-key each iteration until /ais/current-key returns success.
# The signer gRPC service may take a few seconds to initialize after actrix starts.
NONCE_DB="$WORKSPACE_ROOT/database/nonce.db"
MAX_KEY_WAIT=60
KEY_COUNTER=0
echo "Warming up actrix AIS signing key (up to ${MAX_KEY_WAIT}s)..."
jq_status() {
    # Extract .status from JSON, returning "missing" on any error.
    local json="$1"
    if [ -z "$json" ]; then echo "missing"; return; fi
    echo "$json" | jq -r '.status // "missing"' 2>/dev/null || echo "missing"
}

while [ $KEY_COUNTER -lt $MAX_KEY_WAIT ]; do
    # Check if /ais/current-key already has a valid key
    CURRENT_KEY_JSON=$(curl -sf "http://localhost:8081/ais/current-key" 2>/dev/null || true)
    if [ "$(jq_status "$CURRENT_KEY_JSON")" = "success" ]; then
        echo -e "${GREEN}✅ Actrix AIS signing key ready${NC}"
        break
    fi

    # Nonce storage ready? Trigger rotation to accelerate key initialization.
    if [ -f "$NONCE_DB" ] && sqlite3 "$NONCE_DB" ".tables" 2>/dev/null | grep -q "nonce_entries"; then
        ROTATE_RESP=$(curl -sf -X POST "http://localhost:8081/ais/rotate-key" 2>/dev/null || true)
        if [ "$(jq_status "$ROTATE_RESP")" = "success" ]; then
            # Re-check immediately after successful rotation
            CURRENT_KEY_JSON=$(curl -sf "http://localhost:8081/ais/current-key" 2>/dev/null || true)
            if [ "$(jq_status "$CURRENT_KEY_JSON")" = "success" ]; then
                echo -e "${GREEN}✅ Actrix AIS signing key ready${NC}"
                break
            fi
        fi
    fi

    sleep 1
    KEY_COUNTER=$((KEY_COUNTER + 1))
done
if [ $KEY_COUNTER -eq $MAX_KEY_WAIT ]; then
    echo -e "${RED}❌ AIS key warmup timed out after ${MAX_KEY_WAIT}s${NC}"
    grep -aEn "GenerateSigningKey|Authentication failed" "$LOG_DIR/actrix.log" || true
    exit 1
fi

# Extract realm IDs from actr.toml (runtime config) files
SERVER_REALM=1001
CLIENT_REALM=$(grep -E 'realm_id\s*=' "$CLIENT_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')

# Insert realms directly into SQLite (same approach as actrix fullstack tests)
ACTRIX_DB="$WORKSPACE_ROOT/database/actrix.db"

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix database not found at $ACTRIX_DB${NC}"
    echo "Actrix may not have started properly."
    exit 1
fi

for REALM_ID in $SERVER_REALM $CLIENT_REALM; do
    echo "  Creating realm $REALM_ID..."
    sqlite3 "$ACTRIX_DB" \
        "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES ($REALM_ID, 'package-echo-realm', 'Active', 1, strftime('%s','now'), '');"
done

echo -e "${GREEN}✅ Realms setup completed (realm IDs: $SERVER_REALM, $CLIENT_REALM)${NC}"

# ── Step 2.6: Register MFR manufacturer identity ────────────────────────

echo ""
echo "🏷️  Registering MFR manufacturer 'actrium' with public key..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

NOW=$(date +%s)
EXPIRES_AT=$((NOW + 86400 * 365))
MFR_PUBKEY=$(jq -r '.public_key' "$PUBLIC_KEY_PATH")

if [ -z "$MFR_PUBKEY" ]; then
    echo -e "${RED}❌ Failed to extract public_key from $PUBLIC_KEY_PATH${NC}"
    exit 1
fi

MFR_KEY_ID=$(printf '%s' "$MFR_PUBKEY" | base64 -d | openssl dgst -sha256 -r | awk '{print "mfr-" substr($1, 1, 16)}')

if [ -z "$MFR_KEY_ID" ]; then
    echo -e "${RED}❌ Failed to compute key_id from $PUBLIC_KEY_PATH${NC}"
    exit 1
fi

sqlite3 "$ACTRIX_DB" << SQL
INSERT OR IGNORE INTO mfr
    (name, public_key, key_id, contact, status, created_at, updated_at, verified_at, key_expires_at)
VALUES ('actrium', '$MFR_PUBKEY', '$MFR_KEY_ID', 'examples@actrium.local', 'active',
        $NOW, $NOW, $NOW, $EXPIRES_AT);
UPDATE mfr
   SET public_key     = '$MFR_PUBKEY',
       key_id         = '$MFR_KEY_ID',
       status         = 'active',
       updated_at     = $NOW,
       verified_at    = $NOW,
       key_expires_at = $EXPIRES_AT,
       suspended_at   = NULL,
       revoked_at     = NULL
 WHERE name = 'actrium';
SQL

echo -e "${GREEN}✅ MFR manufacturer 'actrium' registered (key_id: $MFR_KEY_ID, pubkey: ${MFR_PUBKEY:0:20}...)${NC}"

# ── Step 2.7: Publish package via actr pkg publish ───────────────────────

echo ""
echo "📡 Publishing package to MFR (challenge-response flow)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg publish \
    --package "$ACTR_PACKAGE" \
    --keychain "$SIGNING_KEY" \
    --endpoint "http://localhost:8081"

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ actr pkg publish failed${NC}"
    echo "Actrix logs (last 20 lines):"
    tail -20 "$LOG_DIR/actrix.log"
    exit 1
fi

echo -e "${GREEN}✅ Package published via challenge-response nonce flow${NC}"

# ── Step 2.8: Seed client identity for AIS Path 1 lookup ─────────────────

echo ""
echo "🪪 Seeding client package identity for AIS lookup..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

CLIENT_MANUFACTURER="acme"
CLIENT_NAME="package-echo-client-app"
CLIENT_VERSION="0.1.0"
CLIENT_TYPE_STR="$CLIENT_MANUFACTURER:$CLIENT_NAME:$CLIENT_VERSION"

sqlite3 "$ACTRIX_DB" << SQL
INSERT OR IGNORE INTO mfr
    (name, public_key, key_id, contact, status, created_at, updated_at, verified_at)
VALUES ('$CLIENT_MANUFACTURER', '', '', 'examples@actrium.local', 'active', $NOW, $NOW, $NOW);

INSERT OR REPLACE INTO mfr_package
    (mfr_id, manufacturer, name, version, type_str, target, manifest, signature, status, published_at, revoked_at)
SELECT id, '$CLIENT_MANUFACTURER', '$CLIENT_NAME', '$CLIENT_VERSION', '$CLIENT_TYPE_STR', 'native', '', '', 'active', $NOW, NULL
  FROM mfr
 WHERE name = '$CLIENT_MANUFACTURER';
SQL

echo -e "${GREEN}✅ Client package identity seeded: $CLIENT_TYPE_STR${NC}"

# ── Step 2.7: Seed client-guest MFR package metadata ────────────────────

echo ""
echo "🏷️  Seeding client-guest MFR package metadata..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

python3 - <<'PY' "$ACTRIX_DB" "$CLIENT_GUEST_PACKAGE" "$CLIENT_GUEST_PUBLIC_KEY"
import base64
import json
import sqlite3
import sys
import time
import tomllib
import zipfile

db_path, package_path, public_key_path = sys.argv[1:]
now = int(time.time())
key_expires_at = now + 365 * 24 * 3600

with open(public_key_path, "r", encoding="utf-8") as fh:
    public_key = json.load(fh)["public_key"]

with zipfile.ZipFile(package_path, "r") as zf:
    manifest = zf.read("manifest.toml").decode("utf-8")
    signature = base64.b64encode(zf.read("manifest.sig")).decode("ascii")

manifest_data = tomllib.loads(manifest)
manufacturer = manifest_data["manufacturer"]
name = manifest_data["name"]
version = manifest_data["version"]
target = manifest_data.get("binary", {}).get("target", "cdylib")
type_str = f"{manufacturer}:{name}:{version}"

conn = sqlite3.connect(db_path)
try:
    cur = conn.cursor()
    cur.execute(
        """
        INSERT OR IGNORE INTO mfr
            (name, public_key, contact, status, created_at, updated_at, verified_at, key_expires_at)
        VALUES (?, ?, ?, 'active', ?, ?, ?, ?)
        """,
        (manufacturer, public_key, "examples@actrium.local", now, now, now, key_expires_at),
    )
    cur.execute(
        """
        UPDATE mfr
           SET public_key = ?,
               status = 'active',
               updated_at = ?,
               verified_at = ?,
               key_expires_at = ?,
               suspended_at = NULL,
               revoked_at = NULL
         WHERE name = ?
        """,
        (public_key, now, now, key_expires_at, manufacturer),
    )
    cur.execute("SELECT id FROM mfr WHERE name = ?", (manufacturer,))
    row = cur.fetchone()
    if row is None:
        raise RuntimeError(f"failed to seed manufacturer {manufacturer}")
    mfr_id = row[0]

    cur.execute(
        """
        INSERT INTO mfr_package
            (mfr_id, manufacturer, name, version, type_str, target, manifest, signature, status, published_at, revoked_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, NULL)
        ON CONFLICT(manufacturer, name, version, target) DO UPDATE SET
            mfr_id = excluded.mfr_id,
            type_str = excluded.type_str,
            manifest = excluded.manifest,
            signature = excluded.signature,
            status = 'active',
            published_at = excluded.published_at,
            revoked_at = NULL
        """,
        (mfr_id, manufacturer, name, version, type_str, target, manifest, signature, now),
    )
    conn.commit()
finally:
    conn.close()

print(f"Seeded MFR package metadata for {type_str}")
PY

echo -e "${GREEN}✅ MFR package metadata seeded for client-guest package${NC}"

# ── Step 3: Build host binaries ──────────────────────────────────────────

echo ""
echo "🔨 Building host binaries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin package-echo-client 2>&1; then
    echo -e "${RED}❌ Failed to build binaries${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binaries built successfully${NC}"

# ── Step 4: Start package-backed echo server via actr run ─────────────────

echo ""
echo "🚀 Starting package-echo-server via actr run..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Generate server actr.toml with the actual package path and trust mode
SERVER_ACTR_CONFIG="$PACKAGE_ECHO_DIR/server-actr.toml"
cat > "$SERVER_ACTR_CONFIG" << TOML
edition = 1

[package]
path = "$ACTR_PACKAGE"

[signaling]
url = "ws://localhost:8081/signaling/ws"

[ais_endpoint]
url = "http://localhost:8081/ais"

[deployment]
realm_id = 1001
trust_mode = "production"

[discovery]
visible = true

[observability]
filter_level = "info"
tracing_enabled = false
tracing_endpoint = "http://localhost:4317"
tracing_service_name = "package-echo-server"

[webrtc]
force_relay = false
stun_urls = ["stun:localhost:3478"]
turn_urls = ["turn:localhost:3478"]

[acl]

[[acl.rules]]
permission = "allow"
type = "acme:package-echo-client-guest:0.1.0"
TOML

ACTR_CLI_BIN="$ACTR_REPO_DIR/target/debug/actr"
if [ ! -x "$ACTR_CLI_BIN" ]; then
    echo "Building actr CLI..."
    cargo build --manifest-path "$ACTR_CLI_MANIFEST" --bin actr 2>&1 | tail -3
fi

RUST_LOG="${RUST_LOG:-info}" \
"$ACTR_CLI_BIN" run -c "$SERVER_ACTR_CONFIG" > "$LOG_DIR/package-echo-server.log" 2>&1 &
SERVER_PID=$!

echo "Server started (PID: $SERVER_PID)"
echo "Waiting for package-backed server to register..."

MAX_WAIT=15
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}❌ Package-backed server failed to start${NC}"
        cat "$LOG_DIR/package-echo-server.log"
        exit 1
    fi

    if grep -q "Echo Host fully started\|ActrNode started" "$LOG_DIR/package-echo-server.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Package-backed server is running and registered${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Server may not have fully registered, but continuing...${NC}"
fi

sleep 2

# ── Step 5: Run client with test input ───────────────────────────────────

echo ""
echo "🚀 Running package-echo-client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ -n "$1" ]; then
    TEST_INPUT="$1"
else
    TEST_INPUT="TestMsg"
fi

echo "Sending test message: \"$TEST_INPUT\""

(
    sleep 3
    echo "$TEST_INPUT"
    sleep 2
    echo "quit"
) | ECHO_ACTR_VERSION="$ECHO_ACTR_VERSION" \
    CLIENT_GUEST_PACKAGE_PATH="$CLIENT_GUEST_PACKAGE" \
    CLIENT_GUEST_PUBLIC_KEY_PATH="$CLIENT_GUEST_PUBLIC_KEY" \
    RUST_LOG="${RUST_LOG:-info}" \
    cargo run --bin package-echo-client > "$LOG_DIR/package-echo-client.log" 2>&1 &
CLIENT_PID=$!

# Wait long enough for the WebRTC connection factory to use its built-in retry path.
# A single attempt can take 10s, and the factory may back off before retrying.
CLIENT_TIMEOUT_SECONDS="${CLIENT_TIMEOUT_SECONDS:-40}"
COUNTER=0
while kill -0 $CLIENT_PID 2>/dev/null && [ $COUNTER -lt "$CLIENT_TIMEOUT_SECONDS" ]; do
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if kill -0 $CLIENT_PID 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Client still running after ${CLIENT_TIMEOUT_SECONDS} seconds, killing...${NC}"
    kill $CLIENT_PID 2>/dev/null || true
fi

# ── Step 6: Verify output ────────────────────────────────────────────────

echo ""
echo "🔍 Verifying output..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if grep -q "\[Received reply\].*Echo: $TEST_INPUT" "$LOG_DIR/package-echo-client.log"; then
    echo -e "${GREEN}✅ Test PASSED: package-backed echo server response received${NC}"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🎉 Echo package test completed successfully!"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "✅ Validated:"
    echo "   1. WASM compiled from $ECHO_ACTR_DIR"
    echo "   2. actr pkg build → signed .actr (key: $SIGNING_KEY)"
    echo "   3. actr pkg publish → registered via challenge-response nonce"
    echo "   4. Hyper (TRUST_MODE=production) fetched pubkey from MFR, verified .actr"
    echo "   5. AIS issued credential (Path 1: published package lookup)"
    echo "   6. Client ↔ server echo confirmed via WebRTC"
    echo ""
    echo "Client output:"
    grep "Received reply" "$LOG_DIR/package-echo-client.log" || true
    echo ""
    echo "📖 View full logs:"
    echo "   cat $LOG_DIR/package-echo-client.log  # Client logs"
    echo "   cat $LOG_DIR/package-echo-server.log  # Server logs"
    echo "   tail -f $LOG_DIR/actrix.log           # Actrix logs"
    echo ""
    exit 0
else
    echo -e "${RED}❌ Test FAILED: Expected package-backed echo server response not found${NC}"
    echo -e "${RED}   Looking for: [Received reply] Echo: $TEST_INPUT${NC}"
    echo ""
    echo "Client output:"
    cat "$LOG_DIR/package-echo-client.log"
    echo ""
    echo "Server output (last 30 lines):"
    tail -30 "$LOG_DIR/package-echo-server.log"
    echo ""
    echo "Actrix output (last 30 lines):"
    tail -30 "$LOG_DIR/actrix.log"
    exit 1
fi
