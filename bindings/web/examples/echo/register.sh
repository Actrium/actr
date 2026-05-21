#!/usr/bin/env bash
# register.sh — Seed realm + MFR manufacturer and publish web echo .actr packages
#               to a running actrix instance.
# Usage: ./register.sh [--db <path>] [--endpoint <url>]
#
# Prerequisites:
#   1. actrix running (signaling + AIS + MFR) on port 8081
#   2. .actr packages already built in release/ (via start.sh Steps 1–2)
#   3. MFR signing key at release/dev-key.json (generated during build)

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"

# ── Defaults ─────────────────────────────────────────────────────────────────

_detect_actrix_db() {
    # First: ask the live process
    if command -v lsof >/dev/null 2>&1; then
        local live_db
        live_db=$(lsof -c actrix 2>/dev/null | awk '/actrix\.db$/ {print $NF; exit}')
        if [ -n "$live_db" ] && [ -f "$live_db" ]; then
            echo "$live_db"
            return
        fi
    fi

    # Fallback: check local dev db first, then candidates
    local candidates=(
        "$SCRIPT_DIR/actrix-dev-db/actrix.db"
        "$(cd "$ACTR_ROOT/.." && pwd)/actrix/database/actrix.db"
        "$(cd "$ACTR_ROOT/.." && pwd)/actrix/actrix-dev-db/actrix.db"
    )
    local best="" best_mtime=0
    for db in "${candidates[@]}"; do
        if [ -f "$db" ] && sqlite3 "$db" ".tables" 2>/dev/null | grep -q "mfr"; then
            local mtime
            mtime=$(stat -f "%m" "$db" 2>/dev/null || stat -c "%Y" "$db" 2>/dev/null || echo 0)
            if [ "$mtime" -gt "$best_mtime" ]; then
                best_mtime="$mtime"
                best="$db"
            fi
        fi
    done
    echo "${best:-$SCRIPT_DIR/actrix-dev-db/actrix.db}"
}

ACTRIX_DB="${ACTRIX_DB:-$(_detect_actrix_db)}"
ENDPOINT="${ENDPOINT:-http://localhost:8081}"
RELEASE_DIR="$SCRIPT_DIR/release"
MFR_KEY_FILE="$RELEASE_DIR/dev-key.json"
MFR_NAME="acme"

# .actr packages (Phase 3 Component Model build — wasm32-wasip2)
SERVER_ACTR_PACKAGE="$RELEASE_DIR/acme-EchoService-0.1.0-wasm32-wasip2.actr"
CLIENT_ACTR_PACKAGE="$RELEASE_DIR/acme-echo-client-app-0.1.0-wasm32-wasip2.actr"

# Config files
SERVER_ACTR_TOML="$SCRIPT_DIR/server-actr.toml"
CLIENT_ACTR_TOML="$SCRIPT_DIR/client-actr.toml"

# Detect actr CLI
ACTR_CMD=""
if [ -x "$ACTR_ROOT/target/debug/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/debug/actr"
elif [ -x "$ACTR_ROOT/target/release/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/release/actr"
elif command -v actr > /dev/null 2>&1; then
    ACTR_CMD="actr"
else
    echo -e "${RED}❌ actr CLI not found${NC}"
    exit 1
fi

# ── Argument parsing ──────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --db)       ACTRIX_DB="$2";  shift 2 ;;
        --endpoint) ENDPOINT="$2";   shift 2 ;;
        *) echo -e "${RED}Unknown argument: $1${NC}"; exit 1 ;;
    esac
done

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  actr register — Web Echo MFR setup"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  DB:       $ACTRIX_DB"
echo "  Endpoint: $ENDPOINT"
echo "  actr:     $ACTR_CMD"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Preflight checks ──────────────────────────────────────────────────────────

if [ ! -f "$ACTRIX_DB" ]; then
    echo -e "${RED}❌ Actrix database not found: $ACTRIX_DB${NC}"
    echo "   Make sure actrix is running and has created its database."
    exit 1
fi

if [ ! -f "$MFR_KEY_FILE" ]; then
    echo -e "${RED}❌ MFR key not found: $MFR_KEY_FILE${NC}"
    echo "   Build packages first (./start.sh builds them)."
    exit 1
fi

if [ ! -f "$SERVER_ACTR_PACKAGE" ]; then
    echo -e "${RED}❌ Server package not found: $SERVER_ACTR_PACKAGE${NC}"
    exit 1
fi

if [ ! -f "$CLIENT_ACTR_PACKAGE" ]; then
    echo -e "${RED}❌ Client package not found: $CLIENT_ACTR_PACKAGE${NC}"
    exit 1
fi

if ! command -v jq >/dev/null 2>&1 && ! command -v python3 >/dev/null 2>&1; then
    echo -e "${RED}❌ jq or python3 is required${NC}"
    exit 1
fi

# Extract MFR public key
if command -v jq >/dev/null 2>&1; then
    MFR_PUBKEY=$(jq -r '.public_key' "$MFR_KEY_FILE")
else
    MFR_PUBKEY=$(python3 -c "import json; print(json.load(open('$MFR_KEY_FILE'))['public_key'])")
fi

if [ -z "$MFR_PUBKEY" ]; then
    echo -e "${RED}❌ Failed to extract public_key from $MFR_KEY_FILE${NC}"
    exit 1
fi

# ── Step 1: Create realm ──────────────────────────────────────────────────────

echo ""
echo "🔑 Step 1: Creating realms..."

SERVER_REALM=$(grep -E 'realm_id\s*=' "$SERVER_ACTR_TOML" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
CLIENT_REALM=$(grep -E 'realm_id\s*=' "$CLIENT_ACTR_TOML" | head -1 | sed 's/.*=\s*//' | tr -d ' ')

NOW=$(date +%s)

for REALM_ID in $SERVER_REALM $CLIENT_REALM; do
    sqlite3 "$ACTRIX_DB" \
        "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) \
         VALUES ($REALM_ID, 'web-echo-realm', 'Active', 1, $NOW, '');"
done

echo -e "${GREEN}✅ Realms ready: $SERVER_REALM, $CLIENT_REALM${NC}"

# ── Step 2: Register MFR manufacturer ────────────────────────────────────────

echo ""
echo "🏷️  Step 2: Registering MFR manufacturer '$MFR_NAME'..."

EXPIRES_AT=$((NOW + 86400 * 365))

sqlite3 "$ACTRIX_DB" << SQL
INSERT OR IGNORE INTO mfr
    (name, public_key, contact, status, created_at, verified_at, key_expires_at)
VALUES ('$MFR_NAME', '$MFR_PUBKEY', 'dev@example.com', 'active',
        $NOW, $NOW, $EXPIRES_AT);
UPDATE mfr
   SET public_key     = '$MFR_PUBKEY',
       status         = 'active',
       updated_at     = $NOW,
       verified_at    = $NOW,
       key_expires_at = $EXPIRES_AT,
       suspended_at   = NULL,
       revoked_at     = NULL
 WHERE name = '$MFR_NAME';
SQL

echo -e "${GREEN}✅ MFR '$MFR_NAME' registered${NC}"

# ── Step 3: Publish packages via actr pkg publish ─────────────────────────────

echo ""
echo "📡 Step 3: Publishing packages to MFR..."

PUBLISH_MAX_RETRIES=5

for PKG_LABEL in server client; do
    if [ "$PKG_LABEL" = "server" ]; then
        PKG_FILE="$SERVER_ACTR_PACKAGE"
    else
        PKG_FILE="$CLIENT_ACTR_PACKAGE"
    fi

    PUBLISH_RETRY=0
    PUBLISH_OK=0
    while [ $PUBLISH_RETRY -lt $PUBLISH_MAX_RETRIES ]; do
        if $ACTR_CMD pkg publish \
            --package "$PKG_FILE" \
            --keychain "$MFR_KEY_FILE" \
            --endpoint "$ENDPOINT"; then
            PUBLISH_OK=1
            break
        fi
        PUBLISH_RETRY=$((PUBLISH_RETRY + 1))
        echo -e "${YELLOW}⚠️  $PKG_LABEL publish failed (attempt $PUBLISH_RETRY/$PUBLISH_MAX_RETRIES), retrying in 2s...${NC}"
        sleep 2
    done
    if [ $PUBLISH_OK -eq 0 ]; then
        echo -e "${RED}❌ ${PKG_LABEL^} package publish failed after $PUBLISH_MAX_RETRIES attempts${NC}"
        exit 1
    fi
    echo -e "${GREEN}✅ ${PKG_LABEL^} package published${NC}"
done

# ── Step 4: Inject MFR pubkey into config files ──────────────────────────────

echo ""
echo "🔐 Step 4: Injecting MFR pubkey into config files..."

sed -i '' "s|__MFR_PUBKEY_PLACEHOLDER__|${MFR_PUBKEY}|g" "$SERVER_ACTR_TOML"
sed -i '' "s|__MFR_PUBKEY_PLACEHOLDER__|${MFR_PUBKEY}|g" "$CLIENT_ACTR_TOML"

echo -e "${GREEN}✅ MFR pubkey injected${NC}"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "${GREEN}✅ Registration complete. You can now run:${NC}"
echo "   actr run --web -c server-actr.toml"
echo "   actr run --web -c client-actr.toml"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
