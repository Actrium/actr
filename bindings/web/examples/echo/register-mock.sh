#!/usr/bin/env bash
# register-mock.sh — Seed realm + MFR + publish web echo .actr packages
#                    against a running `mock-actrix` instance.
#
# Unlike register.sh, this does NOT touch SQLite. It drives the mock's
# `/admin/*` HTTP endpoints instead, which is closer to how a production-grade
# actrix would expose management APIs and decouples the e2e flow from actrix's
# sqlite schema.
#
# Usage: ./register-mock.sh [--endpoint URL]

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ACTR_ROOT="$(cd "$PROJECT_ROOT/../.." && pwd)"

ENDPOINT="${ENDPOINT:-http://127.0.0.1:8081}"
RELEASE_DIR="$SCRIPT_DIR/release"
MFR_KEY_FILE="$RELEASE_DIR/dev-key.json"
MFR_NAME="acme"

SERVER_ACTR_PACKAGE="$RELEASE_DIR/acme-EchoService-0.1.0-wasm32-unknown-unknown.actr"
CLIENT_ACTR_PACKAGE="$RELEASE_DIR/acme-echo-client-app-0.1.0-wasm32-unknown-unknown.actr"

SERVER_ACTR_TOML="$SCRIPT_DIR/server-actr.toml"
CLIENT_ACTR_TOML="$SCRIPT_DIR/client-actr.toml"

ACTR_CMD=""
if [ -x "$ACTR_ROOT/target/debug/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/debug/actr"
elif [ -x "$ACTR_ROOT/target/release/actr" ]; then
    ACTR_CMD="$ACTR_ROOT/target/release/actr"
elif command -v actr > /dev/null 2>&1; then
    ACTR_CMD="actr"
else
    echo -e "${RED}actr CLI not found${NC}"
    exit 1
fi

# --- Argument parsing ---

while [[ $# -gt 0 ]]; do
    case "$1" in
        --endpoint) ENDPOINT="$2"; shift 2 ;;
        *) echo -e "${RED}Unknown argument: $1${NC}"; exit 1 ;;
    esac
done

# Portable in-place sed.
sed_inplace() {
    local expr="$1"; shift
    if sed --version >/dev/null 2>&1; then
        sed -i "$expr" "$@"
    else
        sed -i '' "$expr" "$@"
    fi
}

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  actr register-mock — Web Echo via mock-actrix"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Endpoint: $ENDPOINT"
echo "  actr:     $ACTR_CMD"

# --- Preflight ---

[ -f "$MFR_KEY_FILE" ] || { echo -e "${RED}Missing $MFR_KEY_FILE${NC}"; exit 1; }
[ -f "$SERVER_ACTR_PACKAGE" ] || { echo -e "${RED}Missing server package${NC}"; exit 1; }
[ -f "$CLIENT_ACTR_PACKAGE" ] || { echo -e "${RED}Missing client package${NC}"; exit 1; }

if command -v jq >/dev/null 2>&1; then
    MFR_PUBKEY=$(jq -r '.public_key' "$MFR_KEY_FILE")
else
    MFR_PUBKEY=$(python3 -c "import json; print(json.load(open('$MFR_KEY_FILE'))['public_key'])")
fi
[ -n "$MFR_PUBKEY" ] || { echo -e "${RED}Failed to read MFR pubkey${NC}"; exit 1; }

# --- Step 1: Create realms via /admin/realms ---

echo ""
echo "Step 1: Creating realms..."

SERVER_REALM=$(grep -E 'realm_id\s*=' "$SERVER_ACTR_TOML" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
CLIENT_REALM=$(grep -E 'realm_id\s*=' "$CLIENT_ACTR_TOML" | head -1 | sed 's/.*=\s*//' | tr -d ' ')

for REALM_ID in "$SERVER_REALM" "$CLIENT_REALM"; do
    curl -fsS -X POST "$ENDPOINT/admin/realms" \
        -H 'content-type: application/json' \
        --data "{\"id\": $REALM_ID, \"name\": \"web-echo-realm\"}" >/dev/null
done

echo -e "${GREEN}realms ready: $SERVER_REALM, $CLIENT_REALM${NC}"

# --- Step 2: Register MFR via /admin/mfr ---

echo ""
echo "Step 2: Registering MFR '$MFR_NAME'..."

curl -fsS -X POST "$ENDPOINT/admin/mfr" \
    -H 'content-type: application/json' \
    --data "{\"name\": \"$MFR_NAME\", \"pubkey_b64\": \"$MFR_PUBKEY\", \"contact\": \"dev@example.com\"}" >/dev/null

echo -e "${GREEN}MFR '$MFR_NAME' registered${NC}"

# --- Step 3: Publish packages via `actr pkg publish` (hits /mfr/pkg/* routes) ---

echo ""
echo "Step 3: Publishing packages..."

for PKG_LABEL in server client; do
    if [ "$PKG_LABEL" = "server" ]; then
        PKG_FILE="$SERVER_ACTR_PACKAGE"
    else
        PKG_FILE="$CLIENT_ACTR_PACKAGE"
    fi

    if "$ACTR_CMD" pkg publish \
        --package "$PKG_FILE" \
        --keychain "$MFR_KEY_FILE" \
        --endpoint "$ENDPOINT"; then
        echo -e "${GREEN}$PKG_LABEL package published${NC}"
    else
        echo -e "${RED}$PKG_LABEL package publish failed${NC}"
        exit 1
    fi
done

# --- Step 4: Inject MFR pubkey into config files ---

echo ""
echo "Step 4: Injecting MFR pubkey into config files..."

sed_inplace "s|__MFR_PUBKEY_PLACEHOLDER__|${MFR_PUBKEY}|g" "$SERVER_ACTR_TOML"
sed_inplace "s|__MFR_PUBKEY_PLACEHOLDER__|${MFR_PUBKEY}|g" "$CLIENT_ACTR_TOML"

echo -e "${GREEN}MFR pubkey injected${NC}"

# --- Step 5: Sanity-check state ---

echo ""
echo "Step 5: mock-actrix state snapshot:"
curl -fsS "$ENDPOINT/admin/state" | (jq . 2>/dev/null || cat)

echo ""
echo -e "${GREEN}Registration complete.${NC}"
