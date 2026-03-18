#!/bin/bash
# Test script for package-echo example — echo-actr package loaded from release
#
# Demonstrates the full package-driven execution flow:
#   1. Download the echo-actr release package and public key
#   2. Verify the signed .actr archive
#   3. Host server loads the package and picks the executor from package target
#   4. Client discovers the echo service, sends messages, verifies responses
#
# Usage:
#   ./start.sh              # Use default message "TestMsg"
#   ./start.sh "你好世界"    # Send custom message

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Testing package-echo (echo-actr package loader)"
echo "    Using Actrix as signaling server"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Paths ────────────────────────────────────────────────────────────────

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Actrium root is 3 levels up from WORKSPACE_ROOT (examples/rust):
#   examples/rust → actr/examples → actr → Actrium
ACTRIUM_DIR="$(cd "$WORKSPACE_ROOT/../../.." && pwd)"
ACTRIX_DIR="$ACTRIUM_DIR/actrix"
ACTR_REPO_DIR="$ACTRIUM_DIR/actr"
ACTR_CLI_MANIFEST="$ACTR_REPO_DIR/cli/Cargo.toml"
ACTRIX_CONFIG="$WORKSPACE_ROOT/actrix-config.toml"
PACKAGE_ECHO_DIR="$WORKSPACE_ROOT/package-echo"
SERVER_DIR="$PACKAGE_ECHO_DIR/server"
CLIENT_DIR="$PACKAGE_ECHO_DIR/client"
RELEASE_DIR="$PACKAGE_ECHO_DIR/release"

# Ensure ~/.cargo/bin is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required helper scripts
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# Ensure actr.toml files exist
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$SERVER_DIR"
ensure_actr_toml "$CLIENT_DIR"
cp "$SERVER_DIR/Actr.example.toml" "$SERVER_DIR/actr.toml"
cp "$CLIENT_DIR/Actr.example.toml" "$CLIENT_DIR/actr.toml"
echo -e "${GREEN}✅ Synchronized actr.toml from Actr.example.toml${NC}"

# Ensure actrix-config.toml exists
ensure_actrix_config "$WORKSPACE_ROOT"

# ── Clean stale database files ───────────────────────────────────────────
# Remove DB files from previous runs so actrix starts with fresh keys.
# Without this, expired signing keys cause "Invalid credential format" errors.
echo ""
echo "🗑️  Cleaning stale database files..."
rm -rf "$WORKSPACE_ROOT/database"
echo -e "${GREEN}✅ Stale database cleaned${NC}"

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

# ── Step 0: Download echo-actr release package ──────────────────────────

echo ""
echo -e "${BLUE}📦 Downloading echo-actr release...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ECHO_ACTR_REPO="${ECHO_ACTR_REPO:-Actrium/echo-actr}"
ECHO_ACTR_TAG="${ECHO_ACTR_TAG:-v0.1.0}"
ECHO_ACTR_VERSION="${ECHO_ACTR_VERSION:-${ECHO_ACTR_TAG#v}}"
ECHO_ACTR_BACKEND="${ECHO_ACTR_BACKEND:-wasm}"

host_target() {
    rustc -vV | awk '/host:/ {print $2}'
}

case "$ECHO_ACTR_BACKEND" in
    wasm)
        ECHO_ACTR_TARGET="wasm32-unknown-unknown"
        ;;
    cdylib)
        ECHO_ACTR_TARGET="${ECHO_ACTR_TARGET:-$(host_target)}"
        ;;
    *)
        echo -e "${RED}❌ Unsupported ECHO_ACTR_BACKEND: $ECHO_ACTR_BACKEND${NC}"
        echo "Supported values: wasm, cdylib"
        exit 1
        ;;
esac

ACTR_PACKAGE_NAME="actrium-EchoService-${ECHO_ACTR_VERSION}-${ECHO_ACTR_TARGET}.actr"
PUBLIC_KEY_NAME="public-key.json"
ACTR_PACKAGE="$RELEASE_DIR/$ACTR_PACKAGE_NAME"
PUBLIC_KEY_PATH="$RELEASE_DIR/$PUBLIC_KEY_NAME"

download_release_asset() {
    local asset_name="$1"
    local output_path="$2"
    local asset_url="https://github.com/$ECHO_ACTR_REPO/releases/download/$ECHO_ACTR_TAG/$asset_name"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$asset_url" -o "$output_path"
        return 0
    fi

    if command -v gh >/dev/null 2>&1; then
        gh release download "$ECHO_ACTR_TAG" -R "$ECHO_ACTR_REPO" -p "$asset_name" -D "$RELEASE_DIR" --clobber >/dev/null
        return 0
    fi

    echo -e "${RED}❌ Neither curl nor gh is available for downloading release assets${NC}"
    return 1
}

mkdir -p "$RELEASE_DIR"

echo "Release repo: $ECHO_ACTR_REPO"
echo "Release tag:  $ECHO_ACTR_TAG"
echo "Backend:      $ECHO_ACTR_BACKEND"
echo "Target:       $ECHO_ACTR_TARGET"

if [ -f "$ACTR_PACKAGE" ] && [ -f "$PUBLIC_KEY_PATH" ] && [ "${ECHO_ACTR_FORCE_DOWNLOAD:-0}" != "1" ]; then
    echo -e "${GREEN}✅ Using cached release assets from $RELEASE_DIR${NC}"
else
    download_release_asset "$ACTR_PACKAGE_NAME" "$ACTR_PACKAGE"
    download_release_asset "$PUBLIC_KEY_NAME" "$PUBLIC_KEY_PATH"
fi

if [ ! -f "$ACTR_PACKAGE" ]; then
    echo -e "${RED}❌ Release package download failed: $ACTR_PACKAGE not found${NC}"
    exit 1
fi

if [ ! -f "$PUBLIC_KEY_PATH" ]; then
    echo -e "${RED}❌ Public key download failed: $PUBLIC_KEY_PATH not found${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Release package ready: $(du -h "$ACTR_PACKAGE" | cut -f1) ${NC}"
echo -e "${GREEN}✅ Release public key ready${NC}"

cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg verify \
    --pubkey "$PUBLIC_KEY_PATH" \
    --package "$ACTR_PACKAGE" >/dev/null

echo -e "${GREEN}✅ echo-actr release verified: $(du -h "$ACTR_PACKAGE" | cut -f1) ${NC}"

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

$ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "Actrix started (PID: $ACTRIX_PID)"
echo "Waiting for actrix to be ready..."

MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi

    if lsof -i:8081 > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null; then
        echo -e "${GREEN}✅ Actrix is running and listening on port 8081${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix not listening on port 8081 after ${MAX_WAIT} seconds${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# ── Step 2.5: Setup realms ──────────────────────────────────────────────

echo ""
echo "🔑 Setting up realms in actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

sleep 2

# Extract realm IDs from actr.toml files
SERVER_REALM=$(grep -E 'realm_id\s*=' "$SERVER_DIR/actr.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
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

# ── Step 2.6: Seed MFR package metadata ─────────────────────────────────

echo ""
echo "🏷️  Seeding MFR package metadata..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

python3 - <<'PY' "$ACTRIX_DB" "$ACTR_PACKAGE" "$PUBLIC_KEY_PATH"
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
    manifest = zf.read("actr.toml").decode("utf-8")
    signature = base64.b64encode(zf.read("actr.sig")).decode("ascii")

manifest_data = tomllib.loads(manifest)
manufacturer = manifest_data["manufacturer"]
name = manifest_data["name"]
version = manifest_data["version"]
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
            (mfr_id, manufacturer, name, version, type_str, manifest, signature, status, published_at, revoked_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, NULL)
        ON CONFLICT(manufacturer, name, version) DO UPDATE SET
            mfr_id = excluded.mfr_id,
            type_str = excluded.type_str,
            manifest = excluded.manifest,
            signature = excluded.signature,
            status = 'active',
            published_at = excluded.published_at,
            revoked_at = NULL
        """,
        (mfr_id, manufacturer, name, version, type_str, manifest, signature, now),
    )
    conn.commit()
finally:
    conn.close()

print(f"Seeded MFR package metadata for {type_str}")
PY

echo -e "${GREEN}✅ MFR package metadata seeded for echo-actr release${NC}"

# ── Step 3: Build host binaries ─────────────────────────────────────────

echo ""
echo "🔨 Building host binaries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin package-echo-server --bin package-echo-client 2>&1; then
    echo -e "${RED}❌ Failed to build binaries${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binaries built successfully${NC}"

# ── Step 4: Start package-backed echo server ────────────────────────────

echo ""
echo "🚀 Starting package-echo-server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_PACKAGE_PATH="$ACTR_PACKAGE" ACTR_PUBLIC_KEY_PATH="$PUBLIC_KEY_PATH" RUST_LOG="${RUST_LOG:-info}" cargo run --bin package-echo-server > "$LOG_DIR/package-echo-server.log" 2>&1 &
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

    if grep -q "Echo Server fully started\|ActrNode started" "$LOG_DIR/package-echo-server.log" 2>/dev/null; then
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

# ── Step 5: Run client with test input ──────────────────────────────────

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
) | RUST_LOG="${RUST_LOG:-info}" cargo run --bin package-echo-client > "$LOG_DIR/package-echo-client.log" 2>&1 &
CLIENT_PID=$!

# Wait for client to finish (max 15 seconds)
COUNTER=0
while kill -0 $CLIENT_PID 2>/dev/null && [ $COUNTER -lt 15 ]; do
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if kill -0 $CLIENT_PID 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Client still running after 15 seconds, killing...${NC}"
    kill $CLIENT_PID 2>/dev/null || true
fi

# ── Step 6: Verify output ───────────────────────────────────────────────

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
    echo "   • echo-actr release downloaded from GitHub Release"
    echo "   • Release package signature verified with public-key.json"
    echo "   • Hyper loaded the released .actr package for $ECHO_ACTR_TARGET"
    echo "   • ActrNode started with the package-selected executor"
    echo "   • Real distributed Actor communication (client ↔ package-backed server)"
    echo ""
    echo "Client output:"
    cat "$LOG_DIR/package-echo-client.log" | grep "Received reply" || true
    echo ""
    echo "📖 View full logs:"
    echo "   cat $LOG_DIR/package-echo-client.log  # Client logs"
    echo "   cat $LOG_DIR/package-echo-server.log  # Server logs"
    echo "   tail -f $LOG_DIR/actrix.log        # Actrix logs"
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
