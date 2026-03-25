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
CLIENT_GUEST_DIR="$PACKAGE_ECHO_DIR/client-guest"
ECHO_ACTR_DIR="$ACTRIUM_DIR/echo-actr"

# Ensure ~/.cargo/bin is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

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
# Also remove actr.toml files to ensure they are freshly copied from Actr.example.toml
rm -f "$SERVER_DIR/actr.toml" "$CLIENT_DIR/actr.toml" "$CLIENT_GUEST_DIR/actr.toml"
echo -e "${GREEN}✅ Stale database cleaned${NC}"

# Ensure actr.toml files exist
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$SERVER_DIR"
ensure_actr_toml "$CLIENT_DIR"
cp "$SERVER_DIR/Actr.example.toml" "$SERVER_DIR/actr.toml"
cp "$CLIENT_DIR/Actr.example.toml" "$CLIENT_DIR/actr.toml"
cp "$CLIENT_GUEST_DIR/Actr.example.toml" "$CLIENT_GUEST_DIR/actr.toml"
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

# ── Step 0: Build local echo-actr package ────────────────────────────────

echo ""
echo -e "${BLUE}📦 Building local echo-actr package...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

echo_actr_version() {
    python3 - <<'PY' "$ECHO_ACTR_DIR/Cargo.toml"
import pathlib, sys, tomllib
data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
print(data["package"]["version"])
PY
}

ECHO_ACTR_VERSION="${ECHO_ACTR_VERSION:-$(echo_actr_version)}"
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
ACTR_PACKAGE="$ECHO_ACTR_DIR/dist/$ACTR_PACKAGE_NAME"
PUBLIC_KEY_PATH="$ECHO_ACTR_DIR/public-key.json"

if [ ! -d "$ECHO_ACTR_DIR" ]; then
    echo -e "${RED}❌ echo-actr repository not found: $ECHO_ACTR_DIR${NC}"
    exit 1
fi

echo "echo-actr dir: $ECHO_ACTR_DIR"
echo "Version:       $ECHO_ACTR_VERSION"
echo "Backend:       $ECHO_ACTR_BACKEND"
echo "Target:        $ECHO_ACTR_TARGET"

perl -0pi -e "s/type = \"actrium:EchoService:[^\"]+\"/type = \"actrium:EchoService:${ECHO_ACTR_VERSION}\"/g" \
    "$CLIENT_DIR/actr.toml" "$CLIENT_GUEST_DIR/actr.toml"

"$ECHO_ACTR_DIR/packaging/scripts/check-public-key.sh" >/dev/null

if [ "$ECHO_ACTR_BACKEND" = "wasm" ]; then
    WASM_OPT="${WASM_OPT:-wasm-opt}" "$ECHO_ACTR_DIR/packaging/scripts/build-wasm.sh"
else
    "$ECHO_ACTR_DIR/packaging/scripts/build-native.sh" "$ECHO_ACTR_TARGET"
fi

if [ ! -f "$ACTR_PACKAGE" ]; then
    echo -e "${RED}❌ Local package build failed: $ACTR_PACKAGE not found${NC}"
    exit 1
fi

if [ ! -f "$PUBLIC_KEY_PATH" ]; then
    echo -e "${RED}❌ Public key not found: $PUBLIC_KEY_PATH${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Local package ready: $(du -h "$ACTR_PACKAGE" | cut -f1)${NC}"
echo -e "${GREEN}✅ Local public key ready${NC}"

cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg verify \
    --pubkey "$PUBLIC_KEY_PATH" \
    --package "$ACTR_PACKAGE" >/dev/null

echo -e "${GREEN}✅ Local echo-actr package verified: $(du -h "$ACTR_PACKAGE" | cut -f1)${NC}"

# ── Step 0.5: Build client-guest cdylib package ──────────────────────────

echo ""
echo -e "${BLUE}📦 Building client-guest cdylib package...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

CLIENT_GUEST_VERSION="0.1.0"
CLIENT_GUEST_TARGET="$(rustc -vV | awk '/host:/ {print $2}')"
CLIENT_GUEST_DIST_DIR="$CLIENT_GUEST_DIR/dist"
mkdir -p "$CLIENT_GUEST_DIST_DIR"

# Build the cdylib (runs in workspace root, so target is at workspace level)
WORKSPACE_TARGET_DIR="$WORKSPACE_ROOT/target"
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

# Package the client-guest binary
cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr -- pkg build \
    --binary "$CLIENT_GUEST_BINARY" \
    --config "$CLIENT_GUEST_DIR/actr.toml" \
    --key "$CLIENT_GUEST_DEV_KEY" \
    --target "$CLIENT_GUEST_TARGET" \
    --output "$CLIENT_GUEST_PACKAGE"

if [ ! -f "$CLIENT_GUEST_PACKAGE" ]; then
    echo -e "${RED}❌ client-guest package build failed: $CLIENT_GUEST_PACKAGE not found${NC}"
    exit 1
fi

# Extract public key from dev-key.json for client-guest
python3 - <<'PY' "$CLIENT_GUEST_DEV_KEY" "$CLIENT_GUEST_PUBLIC_KEY"
import json, sys
key_path, out_path = sys.argv[1:]
with open(key_path) as f:
    key = json.load(f)
with open(out_path, "w") as f:
    json.dump({"public_key": key["public_key"]}, f)
PY

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

MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi

    if actrix_http_ready && actrix_ice_ready; then
        echo -e "${GREEN}✅ Actrix is running and listening on ports 8081/tcp and 3478/udp${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix not listening on 8081/tcp and 3478/udp after ${MAX_WAIT} seconds${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# ── Step 2.5: Setup realms ──────────────────────────────────────────────

echo ""
echo "🔑 Setting up realms in actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait until nonce auth storage is initialized, then force a verified AIS key load.
MAX_KEY_WAIT=30
KEY_COUNTER=0
NONCE_DB="$WORKSPACE_ROOT/database/nonce.db"
AIS_KEY_DB="$WORKSPACE_ROOT/database/ais_keys.db"
SIGNER_KEY_DB="$WORKSPACE_ROOT/database/signer_keys.db"
echo "Warming up actrix AIS signing key..."
while [ $KEY_COUNTER -lt $MAX_KEY_WAIT ]; do
    NONCE_READY=0
    if [ -f "$NONCE_DB" ] && sqlite3 "$NONCE_DB" ".tables" 2>/dev/null | grep -q "nonce_entries"; then
        NONCE_READY=1
    fi

    if [ $NONCE_READY -eq 1 ]; then
        CURRENT_KEY_JSON=$(curl -sf "http://localhost:8081/ais/current-key" 2>/dev/null || true)
        CURRENT_KEY_STATUS=$(python3 - <<'PY' "$CURRENT_KEY_JSON"
import json, sys
raw = sys.argv[1]
if not raw:
    print("missing")
    raise SystemExit(0)
try:
    data = json.loads(raw)
except Exception:
    print("invalid")
    raise SystemExit(0)
print(data.get("status", "missing"))
PY
)

        if [ "$CURRENT_KEY_STATUS" != "success" ]; then
            curl -sf -X POST "http://localhost:8081/ais/rotate-key" >/dev/null 2>&1 || true
            CURRENT_KEY_JSON=$(curl -sf "http://localhost:8081/ais/current-key" 2>/dev/null || true)
            CURRENT_KEY_STATUS=$(python3 - <<'PY' "$CURRENT_KEY_JSON"
import json, sys
raw = sys.argv[1]
if not raw:
    print("missing")
    raise SystemExit(0)
try:
    data = json.loads(raw)
except Exception:
    print("invalid")
    raise SystemExit(0)
print(data.get("status", "missing"))
PY
)
        fi

        if [ "$CURRENT_KEY_STATUS" = "success" ] \
            && [ -f "$AIS_KEY_DB" ] \
            && [ -f "$SIGNER_KEY_DB" ] \
            && [ "$(sqlite3 "$AIS_KEY_DB" 'select count(*) from current_key;' 2>/dev/null || echo 0)" -ge 1 ] \
            && [ "$(sqlite3 "$SIGNER_KEY_DB" 'select count(*) from keys;' 2>/dev/null || echo 0)" -ge 1 ]; then
            echo -e "${GREEN}✅ Actrix AIS signing key ready${NC}"
            break
        fi
    fi

    sleep 1
    KEY_COUNTER=$((KEY_COUNTER + 1))
done
if [ $KEY_COUNTER -eq $MAX_KEY_WAIT ]; then
    echo -e "${RED}❌ AIS key warmup timed out after ${MAX_KEY_WAIT}s${NC}"
    grep -aEn "Initial KS key load deferred|Background key rotation failed|GenerateSigningKey|Failed to get key record|Authentication failed" "$LOG_DIR/actrix.log" || true
    exit 1
fi

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
target = manifest_data.get("binary", {}).get("target", "wasm32-unknown-unknown")
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

echo -e "${GREEN}✅ MFR package metadata seeded for local echo-actr package${NC}"

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
    manifest = zf.read("actr.toml").decode("utf-8")
    signature = base64.b64encode(zf.read("actr.sig")).decode("ascii")

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

if ! cargo build --bin package-echo-server --bin package-echo-client 2>&1; then
    echo -e "${RED}❌ Failed to build binaries${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binaries built successfully${NC}"

# ── Step 4: Start package-backed echo server ─────────────────────────────

echo ""
echo "🚀 Starting package-echo-server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ECHO_ACTR_VERSION="$ECHO_ACTR_VERSION" \
ACTR_PACKAGE_PATH="$ACTR_PACKAGE" \
ACTR_PUBLIC_KEY_PATH="$PUBLIC_KEY_PATH" \
RUST_LOG="${RUST_LOG:-info}" \
cargo run --bin package-echo-server > "$LOG_DIR/package-echo-server.log" 2>&1 &
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
    echo "   • Local echo-actr package built from /Users/kaito/Project/Actrium/echo-actr"
    echo "   • Local package signature verified with public-key.json"
    echo "   • Hyper loaded the locally built .actr package for $ECHO_ACTR_TARGET"
    echo "   • ActrNode started with the package-selected workload"
    echo "   • Real distributed Actor communication (client ↔ package-backed server)"
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
