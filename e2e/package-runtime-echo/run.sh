#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$SCRIPT_DIR/lib/common.sh"

HTTP_PORT=8081
ICE_PORT=3478
REALM_ID=""
ADMIN_PASSWORD="e2e-test-password"
MANUFACTURER="actrium"
CLIENT_MANUFACTURER="$MANUFACTURER"
CLIENT_GUEST_VERSION="0.1.0"
ACTRIX_BIN="${ACTRIX_BIN:-}"
ACTR_CLI_MANIFEST="$REPO_ROOT/cli/Cargo.toml"
E2E_TARGET_ROOT="$REPO_ROOT/target/e2e-cache/package-runtime-echo"
ACTR_TARGET_DIR="$E2E_TARGET_ROOT/actr-cli"
WORKSPACE_TARGET_DIR="$E2E_TARGET_ROOT/workspace"
TEMP_SERVICE_TARGET_DIR="$E2E_TARGET_ROOT/temp-service"
DEFAULT_MESSAGE="TmpFlow"

BACKEND="cdylib"
TEST_INPUT="$DEFAULT_MESSAGE"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --backend)
            [[ $# -lt 2 ]] && fail "Missing value for --backend"
            BACKEND="$2"
            shift 2
            ;;
        --backend=*)
            BACKEND="${1#--backend=}"
            shift
            ;;
        -*)
            fail "Unknown option: $1"
            ;;
        *)
            TEST_INPUT="$1"
            shift
            ;;
    esac
done

if [[ "$BACKEND" != "cdylib" ]]; then
    fail "Only --backend cdylib is supported in this scenario"
fi

for cmd in cargo curl jq sqlite3 python3 perl rustc lsof; do
    require_cmd "$cmd"
done
ensure_actrix_available "$REPO_ROOT"

RUN_ID="$(date +%Y%m%d-%H%M%S)-$RANDOM"
RUN_DIR="$SCRIPT_DIR/.tmp/run-$RUN_ID"
STATE_DIR="$RUN_DIR/state"
SQLITE_DIR="$STATE_DIR/sqlite"
LOG_DIR="$RUN_DIR/logs"
DIST_DIR="$RUN_DIR/dist"
TMP_SERVICE_ROOT="$RUN_DIR/workspace"
TMP_SERVICE_DIR="$TMP_SERVICE_ROOT/echo-actr-$RANDOM"
ACTRIX_CONFIG_PATH="$RUN_DIR/actrix.toml"
SERVER_RUNTIME_PATH="$RUN_DIR/server-runtime.toml"
CLIENT_RUNTIME_PATH="$RUN_DIR/client-runtime.toml"
ACTRIX_DB="$SQLITE_DIR/actrix.db"
SERVICE_KEYCHAIN="$TMP_SERVICE_DIR/packaging/keys/mfr.keychain.json"
SERVICE_PUBLIC_KEY="$TMP_SERVICE_DIR/public-key.json"
PROVISIONED_KEYCHAIN="$RUN_DIR/mfr.keychain.json"
PROVISIONED_PUBLIC_KEY="$RUN_DIR/mfr-public-key.json"
CLIENT_GUEST_PACKAGE="$DIST_DIR/${CLIENT_MANUFACTURER}-pkg-runtime-echo-client-guest-${CLIENT_GUEST_VERSION}-cdylib.actr"
CLIENT_GUEST_PUBLIC_KEY="$DIST_DIR/public-key.json"

mkdir -p "$SQLITE_DIR" "$LOG_DIR" "$DIST_DIR" "$TMP_SERVICE_ROOT" "$E2E_TARGET_ROOT"

ACTRIX_PID=""
SERVER_PID=""
CLIENT_PID=""
ACTR_CLI_BIN=""
ADMIN_TOKEN=""
SERVICE_PACKAGE=""
SERVICE_VERSION=""
REALM_SECRET=""
HOST_TARGET="$(rustc -vV | awk '/host:/ {print $2}')"

cleanup() {
    local status=$?

    if [ -n "$CLIENT_PID" ] && kill -0 "$CLIENT_PID" 2>/dev/null; then
        kill "$CLIENT_PID" 2>/dev/null || true
    fi
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -n "$ACTRIX_PID" ] && kill -0 "$ACTRIX_PID" 2>/dev/null; then
        kill "$ACTRIX_PID" 2>/dev/null || true
    fi
    wait 2>/dev/null || true

    if [ $status -eq 0 ] && [ "${KEEP_TMP:-0}" != "1" ]; then
        rm -rf "$RUN_DIR"
    else
        echo ""
        echo "Artifacts preserved at: $RUN_DIR"
    fi
}
trap cleanup EXIT INT TERM

run_actr() {
    CARGO_TARGET_DIR="$ACTR_TARGET_DIR" "$ACTR_CLI_BIN" "$@"
}

build_local_actr_cli() {
    section "🔧 Building local actr CLI"
    CARGO_TARGET_DIR="$ACTR_TARGET_DIR" cargo build --manifest-path "$ACTR_CLI_MANIFEST" --bin actr >/dev/null
    ACTR_CLI_BIN="$ACTR_TARGET_DIR/debug/actr"
    [ -x "$ACTR_CLI_BIN" ] || fail "actr CLI binary missing at $ACTR_CLI_BIN"
    success "actr CLI ready: $ACTR_CLI_BIN"
}

render_runtime_configs() {
    render_template \
        "$SCRIPT_DIR/config/actrix.toml" \
        "$ACTRIX_CONFIG_PATH" \
        "__SQLITE_DIR__=$SQLITE_DIR" \
        "__HTTP_PORT__=$HTTP_PORT" \
        "__ICE_PORT__=$ICE_PORT"
}

start_actrix() {
    section "🚀 Starting local actrix"
    kill_listener tcp "$HTTP_PORT"
    kill_listener udp "$ICE_PORT"

    "$ACTRIX_BIN" --config "$ACTRIX_CONFIG_PATH" >"$LOG_DIR/actrix.log" 2>&1 &
    ACTRIX_PID=$!

    if ! wait_for_http_ok "http://127.0.0.1:${HTTP_PORT}/signaling/health" 120; then
        cat "$LOG_DIR/actrix.log" >&2 || true
        fail "actrix did not become healthy on port $HTTP_PORT"
    fi
    success "actrix is healthy on http://127.0.0.1:${HTTP_PORT}"
}

login_admin() {
    section "🔐 Logging into Admin API"
    local response_file="$RUN_DIR/admin-login.json"
    curl -fsS \
        -X POST \
        "http://127.0.0.1:${HTTP_PORT}/admin/api/auth/login" \
        -H 'Content-Type: application/json' \
        -d "{\"password\":\"${ADMIN_PASSWORD}\"}" \
        >"$response_file"
    ADMIN_TOKEN="$(json_field "$response_file" '.token')"
    success "Admin API login succeeded"
}

warmup_ais_key() {
    section "🔑 Warming up AIS signing key"
    local current_key_file="$RUN_DIR/ais-current-key.json"
    local rotate_file="$RUN_DIR/ais-rotate-key.json"
    local attempt=0

    while [ $attempt -lt 60 ]; do
        if curl -fsS "http://127.0.0.1:${HTTP_PORT}/ais/current-key" >"$current_key_file" 2>/dev/null \
            && [ "$(jq -r '.status // "missing"' "$current_key_file" 2>/dev/null)" = "success" ]; then
            success "AIS signing key is ready"
            return 0
        fi

        curl -fsS -X POST "http://127.0.0.1:${HTTP_PORT}/ais/rotate-key" >"$rotate_file" 2>/dev/null || true
        sleep 1
        attempt=$((attempt + 1))
    done

    fail "AIS signing key warmup timed out"
}

ensure_realm() {
    section "🪪 Creating realm via Admin API"
    local create_file="$RUN_DIR/realm-create.json"
    local realm_name="package-runtime-echo-${RUN_ID}"
    curl -fsS \
        -X POST \
        "http://127.0.0.1:${HTTP_PORT}/admin/api/realms" \
        -H "Authorization: Bearer ${ADMIN_TOKEN}" \
        -H 'Content-Type: application/json' \
        -d "{\"name\":\"${realm_name}\",\"enabled\":true,\"expires_at\":0}" \
        >"$create_file"

    REALM_ID="$(json_field "$create_file" '.realm.realm_id')"
    REALM_SECRET="$(json_field "$create_file" '.realm_secret')"

    [ -n "$REALM_ID" ] || fail "Realm creation returned an empty realm id"
    [ -n "$REALM_SECRET" ] || fail "Realm creation returned an empty realm secret"
    success "Realm ${REALM_ID} created"
}

append_workspace_patch() {
    local cargo_toml="$1"
    local repo_path="$REPO_ROOT"

    if ! grep -q '^\[workspace\]' "$cargo_toml"; then
        cat >>"$cargo_toml" <<'EOF'

[workspace]
EOF
    fi

    if grep -q '^\[patch\.crates-io\]' "$cargo_toml"; then
        return 0
    fi

    cat >>"$cargo_toml" <<EOF

[patch.crates-io]
actr = { path = "$repo_path" }
actr-protocol = { path = "$repo_path/core/protocol" }
actr-framework = { path = "$repo_path/core/framework" }
actr-hyper = { path = "$repo_path/core/hyper" }
actr-runtime = { path = "$repo_path/core/runtime" }
actr-config = { path = "$repo_path/core/config" }
actr-service-compat = { path = "$repo_path/core/service-compat" }
actr-runtime-mailbox = { path = "$repo_path/core/runtime-mailbox" }
EOF
}

write_project_keychain_config() {
    local project_dir="$1"
    local keychain_path="$2"
    mkdir -p "$project_dir/.actr"
    cat >"$project_dir/.actr/config.toml" <<EOF
[mfr]
keychain = "$keychain_path"
EOF
}

provision_mfr_keychain() {
    section "🏷️  Provisioning MFR keychain via Admin API"
    local apply_file="$RUN_DIR/mfr-apply.json"
    local approve_file="$RUN_DIR/mfr-approve.json"
    local now
    now="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

    curl -fsS \
        -X POST \
        "http://127.0.0.1:${HTTP_PORT}/admin/api/mfr/apply" \
        -H "Authorization: Bearer ${ADMIN_TOKEN}" \
        -H 'Content-Type: application/json' \
        -d "{\"github_login\":\"${MANUFACTURER}\",\"contact\":\"e2e@local.actr\"}" \
        >"$apply_file"

    local mfr_id
    mfr_id="$(json_field "$apply_file" '.mfr_id')"

    curl -fsS \
        -X POST \
        "http://127.0.0.1:${HTTP_PORT}/admin/api/mfr/admin/${mfr_id}/approve" \
        -H "Authorization: Bearer ${ADMIN_TOKEN}" \
        -H 'Content-Type: application/json' \
        -d '{}' \
        >"$approve_file"

    mkdir -p "$(dirname "$PROVISIONED_KEYCHAIN")"
    jq -n \
        --arg private_key "$(json_field "$approve_file" '.private_key')" \
        --arg public_key "$(json_field "$approve_file" '.certificate.mfr_pubkey')" \
        --arg created_at "$now" \
        '{
          created_at: $created_at,
          note: "E2E manufacturer signing key issued by local actrix admin API",
          private_key: $private_key,
          public_key: $public_key
        }' \
        >"$PROVISIONED_KEYCHAIN"
    chmod 600 "$PROVISIONED_KEYCHAIN" 2>/dev/null || true

    jq -n \
        --arg public_key "$(json_field "$approve_file" '.certificate.mfr_pubkey')" \
        '{ public_key: $public_key }' \
        >"$PROVISIONED_PUBLIC_KEY"

    success "Generated manufacturer keychain for ${MANUFACTURER}"
}

scaffold_service_guest() {
    section "🧱 Scaffolding temporary echo service"
    run_actr init \
        -l rust \
        --template echo \
        --role service \
        --signaling "ws://127.0.0.1:${HTTP_PORT}/signaling/ws" \
        --manufacturer "$MANUFACTURER" \
        "$TMP_SERVICE_DIR"

    append_workspace_patch "$TMP_SERVICE_DIR/Cargo.toml"
    mkdir -p "$(dirname "$SERVICE_KEYCHAIN")"
    cp "$PROVISIONED_KEYCHAIN" "$SERVICE_KEYCHAIN"
    cp "$PROVISIONED_PUBLIC_KEY" "$SERVICE_PUBLIC_KEY"
    write_project_keychain_config "$TMP_SERVICE_DIR" "$SERVICE_KEYCHAIN"

    (
        cd "$TMP_SERVICE_DIR"
        CARGO_TARGET_DIR="$TEMP_SERVICE_TARGET_DIR" run_actr deps install
        CARGO_TARGET_DIR="$TEMP_SERVICE_TARGET_DIR" run_actr gen -l rust
    )

    SERVICE_VERSION="$(
        awk '
            /^\[package\]/ { in_package = 1; next }
            /^\[/ && in_package { exit }
            in_package && $1 == "version" {
                gsub(/"/, "", $3)
                print $3
                exit
            }
        ' "$TMP_SERVICE_DIR/manifest.toml"
    )"

    [ -n "$SERVICE_VERSION" ] || fail "Unable to detect temporary service version"
    success "Temporary echo service ready: version ${SERVICE_VERSION}"
}

build_service_package() {
    section "📦 Building and publishing the server package"
    SERVICE_PACKAGE="$DIST_DIR/${MANUFACTURER}-EchoService-${SERVICE_VERSION}-${HOST_TARGET}.actr"

    (
        cd "$TMP_SERVICE_DIR"
        CARGO_TARGET_DIR="$TEMP_SERVICE_TARGET_DIR" run_actr build \
            -f manifest.toml \
            --key "$SERVICE_KEYCHAIN" \
            --output "$SERVICE_PACKAGE"
    )

    [ -f "$SERVICE_PACKAGE" ] || fail "Server package missing: $SERVICE_PACKAGE"

    run_actr pkg verify --pubkey "$SERVICE_PUBLIC_KEY" --package "$SERVICE_PACKAGE" >/dev/null
    run_actr pkg publish \
        --package "$SERVICE_PACKAGE" \
        --keychain "$SERVICE_KEYCHAIN" \
        --endpoint "http://127.0.0.1:${HTTP_PORT}"

    success "Server package published"
}

client_guest_library_path() {
    case "$(uname)" in
        Darwin)
            printf '%s\n' "$WORKSPACE_TARGET_DIR/debug/libpackage_runtime_echo_client_guest.dylib"
            ;;
        Linux)
            printf '%s\n' "$WORKSPACE_TARGET_DIR/debug/libpackage_runtime_echo_client_guest.so"
            ;;
        *)
            printf '%s\n' "$WORKSPACE_TARGET_DIR/debug/package_runtime_echo_client_guest.dll"
            ;;
    esac
}

build_client_guest_package() {
    section "📦 Building client guest package"

    CARGO_TARGET_DIR="$WORKSPACE_TARGET_DIR" cargo build --manifest-path "$SCRIPT_DIR/client-guest/Cargo.toml" >/dev/null

    local client_guest_binary
    client_guest_binary="$(client_guest_library_path)"
    [ -f "$client_guest_binary" ] || fail "Client guest library missing: $client_guest_binary"

    run_actr build \
        --binary "$client_guest_binary" \
        --config "$SCRIPT_DIR/client-guest/manifest.toml" \
        --key "$PROVISIONED_KEYCHAIN" \
        --target "$HOST_TARGET" \
        --output "$CLIENT_GUEST_PACKAGE"

    [ -f "$CLIENT_GUEST_PACKAGE" ] || fail "Client guest package missing: $CLIENT_GUEST_PACKAGE"

    cp "$PROVISIONED_PUBLIC_KEY" "$CLIENT_GUEST_PUBLIC_KEY"
    run_actr pkg verify --pubkey "$CLIENT_GUEST_PUBLIC_KEY" --package "$CLIENT_GUEST_PACKAGE" >/dev/null
    success "Client guest package ready"
}

seed_client_registry_state() {
    section "🗂️  Seeding client registry metadata"
    python3 - "$ACTRIX_DB" "$CLIENT_GUEST_PACKAGE" "$CLIENT_GUEST_PUBLIC_KEY" <<'PY'
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
        (manufacturer, public_key, "e2e@local.actr", now, now, now, key_expires_at),
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
    mfr_id = cur.fetchone()[0]
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
PY

    success "Client registry state seeded"
}

render_client_runtime_config() {
    render_template \
        "$SCRIPT_DIR/config/client-runtime.toml.tpl" \
        "$CLIENT_RUNTIME_PATH" \
        "__REALM_ID__=$REALM_ID" \
        "__ECHO_SERVICE_VERSION__=$SERVICE_VERSION" \
        "__REALM_SECRET__=$REALM_SECRET"
}

run_server_host() {
    section "🚀 Starting package-backed server host"
    render_template \
        "$SCRIPT_DIR/config/server-runtime.toml.tpl" \
        "$SERVER_RUNTIME_PATH" \
        "__PACKAGE_PATH__=$SERVICE_PACKAGE" \
        "__REALM_ID__=$REALM_ID" \
        "__REALM_SECRET__=$REALM_SECRET"

    RUST_LOG="${RUST_LOG:-info}" \
        run_actr run -c "$SERVER_RUNTIME_PATH" >"$LOG_DIR/server.log" 2>&1 &
    SERVER_PID=$!

    local attempt=0
    while [ $attempt -lt 20 ]; do
        if ! kill -0 "$SERVER_PID" 2>/dev/null; then
            cat "$LOG_DIR/server.log" >&2 || true
            fail "Server host exited early"
        fi

        if grep -q "Echo Host fully started\|ActrNode started" "$LOG_DIR/server.log" 2>/dev/null; then
            success "Server host is running"
            return 0
        fi

        sleep 1
        attempt=$((attempt + 1))
    done

    warn "Server host readiness log not observed, continuing"
}

run_client_and_assert() {
    section "🚀 Running client host"
    render_client_runtime_config
    CARGO_TARGET_DIR="$WORKSPACE_TARGET_DIR" cargo build --manifest-path "$SCRIPT_DIR/client/Cargo.toml" >/dev/null

    (
        sleep 3
        echo "$TEST_INPUT"
        sleep 2
        echo "quit"
    ) | \
        ECHO_ACTR_VERSION="$SERVICE_VERSION" \
        CLIENT_RUNTIME_CONFIG_PATH="$CLIENT_RUNTIME_PATH" \
        CLIENT_GUEST_PACKAGE_PATH="$CLIENT_GUEST_PACKAGE" \
        CLIENT_GUEST_PUBLIC_KEY_PATH="$CLIENT_GUEST_PUBLIC_KEY" \
        RUST_LOG="${RUST_LOG:-info}" \
        CARGO_TARGET_DIR="$WORKSPACE_TARGET_DIR" \
        cargo run --manifest-path "$SCRIPT_DIR/client/Cargo.toml" --bin package-runtime-echo-client \
        >"$LOG_DIR/client.log" 2>&1 &
    CLIENT_PID=$!

    local timeout="${CLIENT_TIMEOUT_SECONDS:-40}"
    local attempt=0
    while kill -0 "$CLIENT_PID" 2>/dev/null && [ $attempt -lt "$timeout" ]; do
        sleep 1
        attempt=$((attempt + 1))
    done

    if kill -0 "$CLIENT_PID" 2>/dev/null; then
        kill "$CLIENT_PID" 2>/dev/null || true
        fail "Client host timed out after ${timeout}s"
    fi

    if grep -q "\[Received reply\].*Echo: ${TEST_INPUT}" "$LOG_DIR/client.log"; then
        success "End-to-end echo succeeded"
        grep "Received reply" "$LOG_DIR/client.log" || true
        return 0
    fi

    echo ""
    echo "Client log:"
    cat "$LOG_DIR/client.log" || true
    echo ""
    echo "Server log:"
    cat "$LOG_DIR/server.log" || true
    echo ""
    echo "Actrix log:"
    cat "$LOG_DIR/actrix.log" || true
    fail "Expected echo reply not found"
}

section "🧪 Package Runtime Echo E2E"
echo "Run directory: $RUN_DIR"
echo "Backend:       $BACKEND"
echo "Message:       $TEST_INPUT"
echo "Actrix binary: $ACTRIX_BIN"

render_runtime_configs
build_local_actr_cli
start_actrix
login_admin
warmup_ais_key
ensure_realm
provision_mfr_keychain
scaffold_service_guest
build_service_package
build_client_guest_package
seed_client_registry_state
run_server_host
run_client_and_assert

echo ""
success "Package runtime echo E2E completed successfully"
