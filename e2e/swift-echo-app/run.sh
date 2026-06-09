#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$SCRIPT_DIR/../package-runtime-echo/lib/common.sh"

HTTP_PORT=8081
ICE_PORT=3478
REALM_ID=""
ADMIN_PASSWORD="e2e-test-password"
MANUFACTURER="actrium"
ACTRIX_BIN="${ACTRIX_BIN:-}"
ACTR_CLI_MANIFEST="$REPO_ROOT/cli/Cargo.toml"
E2E_TARGET_ROOT="$REPO_ROOT/target/e2e-cache/swift-echo-app"
ACTR_TARGET_DIR="$E2E_TARGET_ROOT/actr-cli"
TEMP_SERVICE_TARGET_DIR="$E2E_TARGET_ROOT/temp-service"
SWIFT_PACKAGE_DIR="$REPO_ROOT/bindings/swift"
SWIFT_BINDINGS_PATH=".build/e2e/ActrBindings"
SWIFT_XCFRAMEWORK_PATH=".build/e2e/ActrFFI.xcframework"
DEFAULT_MESSAGE="e2e-test-message"

TEST_INPUT="$DEFAULT_MESSAGE"

while [[ $# -gt 0 ]]; do
    case "$1" in
        -*)
            fail "Unknown option: $1"
            ;;
        *)
            TEST_INPUT="$1"
            shift
            ;;
    esac
done

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
SERVICE_KEYCHAIN="$TMP_SERVICE_DIR/packaging/keys/mfr.keychain.json"
SERVICE_PUBLIC_KEY="$TMP_SERVICE_DIR/public-key.json"
PROVISIONED_KEYCHAIN="$RUN_DIR/mfr.keychain.json"
PROVISIONED_PUBLIC_KEY="$RUN_DIR/mfr-public-key.json"
ECHOAPP_ACTRIX_CONFIG="$SCRIPT_DIR/actr.toml"
HOST_TARGET="$(rustc -vV | awk '/host:/ {print $2}')"
ECHOAPP_PACKAGE_MANIFEST="$RUN_DIR/echoapp-package-manifest.toml"
ECHOAPP_MARKER_BINARY="$RUN_DIR/echoapp-linked-identity.bin"
ECHOAPP_PACKAGE="$DIST_DIR/${MANUFACTURER}-EchoApp-0.1.0-${HOST_TARGET}.actr"
APP_STDOUT_LOG="$LOG_DIR/app.stdout.log"
APP_STDERR_LOG="$LOG_DIR/app.stderr.log"

mkdir -p "$SQLITE_DIR" "$LOG_DIR" "$DIST_DIR" "$TMP_SERVICE_ROOT" "$E2E_TARGET_ROOT"

ACTRIX_PID=""
SERVER_PID=""
ACTR_CLI_BIN=""
ADMIN_TOKEN=""
SERVICE_PACKAGE=""
SERVICE_VERSION=""
REALM_SECRET=""
DEVICE_UDID=""

cleanup() {
    local status=$?

    if [ -n "$DEVICE_UDID" ]; then
        xcrun simctl terminate "$DEVICE_UDID" com.actrium.EchoApp 2>/dev/null || true
    fi
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -n "$ACTRIX_PID" ] && kill -0 "$ACTRIX_PID" 2>/dev/null; then
        kill "$ACTRIX_PID" 2>/dev/null || true
    fi
    wait 2>/dev/null || true

    # Shut down booted iOS Simulators
    xcrun simctl shutdown all 2>/dev/null || true

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

# ──── Rust / actrix lifecycle (reused from package-runtime-echo) ────

build_local_actr_cli() {
    section "🔧 Building local actr CLI"
    local cargo_env=()

    if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists libssh2; then
        cargo_env+=(LIBSSH2_SYS_USE_PKG_CONFIG=1)
    fi

    env "${cargo_env[@]}" CARGO_TARGET_DIR="$ACTR_TARGET_DIR" cargo build --manifest-path "$ACTR_CLI_MANIFEST" --bin actr >/dev/null
    ACTR_CLI_BIN="$ACTR_TARGET_DIR/debug/actr"
    [ -x "$ACTR_CLI_BIN" ] || fail "actr CLI binary missing at $ACTR_CLI_BIN"
    success "actr CLI ready: $ACTR_CLI_BIN"
}

render_runtime_configs() {
    render_template \
        "$SCRIPT_DIR/../package-runtime-echo/config/actrix.toml" \
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
    local realm_name="swift-echo-app-${RUN_ID}"
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
actr-config = { path = "$repo_path/core/config" }
actr-protocol = { path = "$repo_path/core/protocol" }
actr-framework = { path = "$repo_path/core/framework" }
actr-hyper = { path = "$repo_path/core/hyper" }
actr-pack = { path = "$repo_path/core/pack" }
actr-platform-native = { path = "$repo_path/core/platform-native" }
actr-platform-traits = { path = "$repo_path/core/platform-traits" }
actr-runtime = { path = "$repo_path/core/runtime" }
actr-runtime-mailbox = { path = "$repo_path/core/runtime-mailbox" }
actr-service-compat = { path = "$repo_path/core/service-compat" }
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
            --manifest-path manifest.toml \
            --key "$SERVICE_KEYCHAIN" \
            --output "$SERVICE_PACKAGE"
    )

    [ -f "$SERVICE_PACKAGE" ] || fail "Server package missing: $SERVICE_PACKAGE"

    run_actr pkg verify --pubkey "$SERVICE_PUBLIC_KEY" --package "$SERVICE_PACKAGE" >/dev/null
    run_actr registry publish \
        --package "$SERVICE_PACKAGE" \
        --keychain "$SERVICE_KEYCHAIN" \
        --endpoint "http://127.0.0.1:${HTTP_PORT}"

    success "Server package published"
}

publish_echoapp_package_identity() {
    section "📦 Publishing EchoApp package identity"

    # Linked EchoApp does not load this package. It is a registry marker for
    # actrix versions that still require the actor type to be package-registered.
    printf 'linked EchoApp identity marker\n' >"$ECHOAPP_MARKER_BINARY"
    cat >"$ECHOAPP_PACKAGE_MANIFEST" <<EOF
edition = 1

[package]
name = "EchoApp"
manufacturer = "${MANUFACTURER}"
version = "0.1.0"
description = "Actor-RTC EchoApp linked runtime identity marker"

[binary]
path = "${ECHOAPP_MARKER_BINARY}"
target = "${HOST_TARGET}"
EOF

    run_actr build \
        --no-compile \
        --manifest-path "$ECHOAPP_PACKAGE_MANIFEST" \
        --key "$PROVISIONED_KEYCHAIN" \
        --output "$ECHOAPP_PACKAGE"

    run_actr pkg verify --pubkey "$PROVISIONED_PUBLIC_KEY" --package "$ECHOAPP_PACKAGE" >/dev/null
    run_actr registry publish \
        --package "$ECHOAPP_PACKAGE" \
        --keychain "$PROVISIONED_KEYCHAIN" \
        --endpoint "http://127.0.0.1:${HTTP_PORT}"

    success "EchoApp package identity published"
}

run_server_host() {
    section "🚀 Starting package-backed server host"

    cat >"$SERVER_RUNTIME_PATH" <<EOF
edition = 1

[package]
path = "${SERVICE_PACKAGE}"

[signaling]
url = "ws://127.0.0.1:${HTTP_PORT}/signaling/ws"

[ais_endpoint]
url = "http://127.0.0.1:${HTTP_PORT}/ais"

[deployment]
realm_id = ${REALM_ID}
realm_secret = "${REALM_SECRET}"

[[trust]]
kind = "registry"
endpoint = "http://127.0.0.1:${HTTP_PORT}/ais"

[discovery]
visible = true

[observability]
filter_level = "info"
tracing_enabled = false
tracing_endpoint = "http://localhost:4317"
tracing_service_name = "swift-echo-app-server"

[webrtc]
force_relay = false
stun_urls = ["stun:127.0.0.1:${ICE_PORT}"]
turn_urls = ["turn:127.0.0.1:${ICE_PORT}"]

[acl]

[[acl.rules]]
permission = "allow"
type = "${MANUFACTURER}:EchoApp:0.1.0"
EOF

    RUST_LOG="${RUST_LOG:-info}" \
        run_actr run -c "$SERVER_RUNTIME_PATH" >"$LOG_DIR/server.log" 2>&1 &
    SERVER_PID=$!

    local attempt=0
    while [ $attempt -lt 30 ]; do
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

# ──── EchoApp config ────

render_echoapp_config() {
    section "📝 Rendering EchoApp runtime config"
    render_template \
        "$SCRIPT_DIR/actr.toml.tpl" \
        "$ECHOAPP_ACTRIX_CONFIG" \
        "__HOST__=127.0.0.1" \
        "__HTTP_PORT__=$HTTP_PORT" \
        "__ICE_PORT__=$ICE_PORT" \
        "__REALM_ID__=$REALM_ID" \
        "__REALM_SECRET__=$REALM_SECRET"
    success "EchoApp actr.toml rendered"
}

# ──── iOS Simulator ────

build_local_swift_package_assets() {
    section "🧩 Building local Swift FFI package assets"

    require_cmd uniffi-bindgen
    if ! (
        cd "$SWIFT_PACKAGE_DIR"
        ACTR_BUILD_PROFILE="${ACTR_BUILD_PROFILE:-debug}" \
            ACTR_BINDINGS_PATH="$SWIFT_BINDINGS_PATH" \
            ACTR_BINARY_PATH="$SWIFT_XCFRAMEWORK_PATH" \
            ./build-xcframework.sh >"$LOG_DIR/swift-xcframework.log" 2>&1
    ); then
        tail -120 "$LOG_DIR/swift-xcframework.log" >&2 || true
        fail "Swift FFI XCFramework build failed"
    fi

    [ -f "$SWIFT_PACKAGE_DIR/$SWIFT_BINDINGS_PATH/Actr.swift" ] || fail "Swift bindings missing"
    [ -d "$SWIFT_PACKAGE_DIR/$SWIFT_XCFRAMEWORK_PATH" ] || fail "Swift XCFramework missing"
    success "Local Swift FFI package assets ready"
}

setup_ios_simulator() {
    section "📱 Setting up iOS Simulator"

    # Find available iOS runtime
    RUNTIME_ID="$(xcrun simctl list runtimes -j | jq -r '.runtimes[] | select(.name | test("iOS")) | .identifier' | tail -1)"
    [ -n "$RUNTIME_ID" ] || fail "No iOS Simulator runtime found"
    success "iOS runtime: $RUNTIME_ID"

    # Find template device for the runtime
    DEVICE_TYPE_ID="$(xcrun simctl list devicetypes -j | jq -r '.devicetypes[] | select(.name | test("iPhone 16$")) | .identifier' | head -1)"
    if [ -z "$DEVICE_TYPE_ID" ]; then
        DEVICE_TYPE_ID="$(xcrun simctl list devicetypes -j | jq -r '.devicetypes[] | select(.name | test("iPhone")) | .identifier' | tail -1)"
    fi
    [ -n "$DEVICE_TYPE_ID" ] || fail "No iPhone device type found"
    success "Device type: $DEVICE_TYPE_ID"

    # Look for an existing device with this runtime + device type
    DEVICE_UDID="$(xcrun simctl list devices -j | jq -r --arg runtime "$RUNTIME_ID" --arg dt "$DEVICE_TYPE_ID" '
        .devices[$runtime] // [] | .[] | select(.deviceTypeIdentifier == $dt) | .udid' | head -1)"

    if [ -z "$DEVICE_UDID" ]; then
        DEVICE_NAME="swift-echo-e2e-${RUN_ID}"
        DEVICE_UDID="$(xcrun simctl create "$DEVICE_NAME" "$DEVICE_TYPE_ID" "$RUNTIME_ID")"
        success "Created simulator: $DEVICE_NAME ($DEVICE_UDID)"
    else
        success "Reusing simulator: $DEVICE_UDID"
    fi

    xcrun simctl boot "$DEVICE_UDID" 2>/dev/null || true
    if xcrun simctl bootstatus "$DEVICE_UDID" -b >/dev/null 2>&1; then
        success "Simulator booted"
        export DEVICE_UDID
        return 0
    fi

    # Fall back to polling the device state when bootstatus is unavailable or flaky.
    local attempt=0
    while [ $attempt -lt 60 ]; do
        local boot_status
        boot_status="$(xcrun simctl list devices -j | jq -r --arg udid "$DEVICE_UDID" '
            .devices | to_entries | .[] | .value | .[] | select(.udid == $udid) | .state')"
        if [ "$boot_status" = "Booted" ]; then
            success "Simulator booted"
            break
        fi
        sleep 1
        attempt=$((attempt + 1))
    done

    fail "Simulator did not boot: $DEVICE_UDID"

    export DEVICE_UDID
}

build_and_run_app() {
    section "🔨 Building EchoApp with XcodeGen"

    require_cmd xcodegen
    build_local_swift_package_assets
    cd "$SCRIPT_DIR"

    # Generate Xcode project from project.yml
    rm -rf EchoApp.xcodeproj
    xcodegen generate --spec project.yml --project "$SCRIPT_DIR" >"$LOG_DIR/xcodegen.log" 2>&1
    success "XcodeGen project generated"

    section "🏗️  Building EchoApp for iOS Simulator"

    local derived_data="$RUN_DIR/DerivedData"
    ACTR_BINDINGS_PATH="$SWIFT_BINDINGS_PATH" \
    ACTR_BINARY_PATH="$SWIFT_XCFRAMEWORK_PATH" \
    xcodebuild \
        -project EchoApp.xcodeproj \
        -scheme EchoApp \
        -destination "id=$DEVICE_UDID" \
        -derivedDataPath "$derived_data" \
        -configuration Debug \
        build \
        2>&1 | tee "$LOG_DIR/xcodebuild.log"

    # Find built .app
    APP_PATH="$(find "$derived_data/Build/Products" -name "EchoApp.app" -type d | head -1)"
    [ -n "$APP_PATH" ] || {
        tail -100 "$LOG_DIR/xcodebuild.log" >&2
        fail "EchoApp.app not found in build products"
    }
    success "App built: $APP_PATH"

    section "📲 Installing and launching EchoApp"
    xcrun simctl install "$DEVICE_UDID" "$APP_PATH"

    # Launch with direct stdout/stderr redirection. `simctl launch --console`
    # may return before the app exits when detached from the terminal, so do not
    # treat the wrapper process as the app lifetime.
    SIMCTL_CHILD_ACTR_ECHOAPP_AUTO_SEND=1 \
    SIMCTL_CHILD_ACTR_ECHOAPP_TEST_INPUT="$TEST_INPUT" \
    xcrun simctl launch \
        --terminate-running-process \
        --stdout="$APP_STDOUT_LOG" \
        --stderr="$APP_STDERR_LOG" \
        "$DEVICE_UDID" \
        "com.actrium.EchoApp" \
        >"$LOG_DIR/app.launch.log" 2>&1

    success "App launched, waiting for echo result"
}

grep_app_logs() {
    grep -h "$@" "$APP_STDOUT_LOG" "$APP_STDERR_LOG" 2>/dev/null
}

tail_app_logs() {
    local lines="$1"
    echo "App stdout log tail:"
    tail -n "$lines" "$APP_STDOUT_LOG" >&2 2>/dev/null || true
    echo "App stderr log tail:"
    tail -n "$lines" "$APP_STDERR_LOG" >&2 2>/dev/null || true
}

wait_for_echo_result() {
    section "⏳ Waiting for echo result"
    local timeout="${CLIENT_TIMEOUT_SECONDS:-120}"
    local attempt=0

    while [ $attempt -lt "$timeout" ]; do
        if grep_app_logs -q "ACTR_E2E_RESULT:"; then
            local result
            result="$(grep_app_logs "ACTR_E2E_RESULT:" | tail -1)"
            echo "Echo result: $result"
            if echo "$result" | grep -q "$TEST_INPUT"; then
                success "End-to-end echo succeeded"
                return 0
            fi
            warn "Echo result received but does not contain expected message: $TEST_INPUT"
            return 1
        fi

        sleep 2
        attempt=$((attempt + 1))
    done

    echo ""
    tail_app_logs 80
    fail "Timed out waiting for echo result after ${timeout}s"
}

# ──── Main ────

section "🧪 Swift EchoApp E2E"
echo "Run directory: $RUN_DIR"
echo "Message:       $TEST_INPUT"
echo "Actrix binary: $ACTRIX_BIN"
echo "Host target:   $HOST_TARGET"

render_runtime_configs
build_local_actr_cli
start_actrix
login_admin
warmup_ais_key
ensure_realm
provision_mfr_keychain
scaffold_service_guest
build_service_package
publish_echoapp_package_identity
run_server_host

render_echoapp_config
setup_ios_simulator
build_and_run_app
wait_for_echo_result

echo ""
success "Swift EchoApp e2e completed successfully"
