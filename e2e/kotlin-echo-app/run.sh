#!/usr/bin/env bash
#
# kotlin-echo-app/run.sh — Kotlin Android nightly e2e (echo scenario).
#
# Mirrors e2e/swift-echo-app: a Rust EchoService is scaffolded, built, published
# to local actrix and run; a Kotlin Android client (linked mode, scaffolded via
# `actr init -l kotlin --template echo`) calls echo.EchoService.Echo on it; the
# EchoIntegrationTest asserts the reply. Verification is `./gradlew
# connectedDebugAndroidTest` on an already-booted Android emulator (CI: booted by
# reactivecircus/android-emulator-runner; locally: boot one beforehand).
#
# Heavy work (native .so + AAR publish) is done up front to avoid an idle emulator.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$REPO_ROOT/e2e/package-runtime-echo/lib/common.sh"
source "$SCRIPT_DIR/../kotlin-lib/lib/readiness.sh"
source "$SCRIPT_DIR/../kotlin-lib/lib/kotlin-app.sh"

HTTP_PORT=8081
ICE_PORT=3478
ADMIN_PASSWORD="e2e-test-password"
MANUFACTURER="actrium"
ACTR_CLI_MANIFEST="$REPO_ROOT/cli/Cargo.toml"
ACTRIX_CONFIG_TEMPLATE="$REPO_ROOT/e2e/package-runtime-echo/config/actrix.toml"
E2E_TARGET_ROOT="$REPO_ROOT/target/e2e-cache/kotlin-echo-app"
ACTR_TARGET_DIR="$E2E_TARGET_ROOT/actr-cli"
TEMP_SERVICE_TARGET_DIR="$E2E_TARGET_ROOT/temp-service"

REALM_NAME_PREFIX="kotlin-echo-app"
SERVICE_ACTOR="EchoService"
APP_ACTOR="KotlinEchoApp"
APP_VERSION="0.1.0"
APPLICATION_ID="io.actrium.kotlinechoapp"

for cmd in cargo curl jq sqlite3 python3 perl rustc lsof adb; do
    require_cmd "$cmd"
done

RUN_ID="$(date +%Y%m%d-%H%M%S)-$RANDOM"
RUN_DIR="$SCRIPT_DIR/.tmp/run-$RUN_ID"
STATE_DIR="$RUN_DIR/state"
SQLITE_DIR="$STATE_DIR/sqlite"
LOG_DIR="$RUN_DIR/logs"
DIST_DIR="$RUN_DIR/dist"
TMP_SERVICE_ROOT="$RUN_DIR/workspace"
TMP_SERVICE_DIR="$TMP_SERVICE_ROOT/echo-actr-$RANDOM"
TMP_APP_DIR="$RUN_DIR/app"
ACTRIX_CONFIG_PATH="$RUN_DIR/actrix.toml"
SERVER_RUNTIME_PATH="$RUN_DIR/server-runtime.toml"
SERVICE_KEYCHAIN="$TMP_SERVICE_DIR/packaging/keys/mfr.keychain.json"
SERVICE_PUBLIC_KEY="$TMP_SERVICE_DIR/public-key.json"
PROVISIONED_KEYCHAIN="$RUN_DIR/mfr.keychain.json"
PROVISIONED_PUBLIC_KEY="$RUN_DIR/mfr-public-key.json"

mkdir -p "$SQLITE_DIR" "$LOG_DIR" "$DIST_DIR" "$TMP_SERVICE_ROOT" "$TMP_APP_DIR" "$E2E_TARGET_ROOT"

ACTRIX_PID=""
SERVER_PID=""
ACTR_CLI_BIN=""
ACTRIX_BIN=""
ADMIN_TOKEN=""
SERVICE_PACKAGE=""
SERVICE_VERSION=""
REALM_ID=""
REALM_SECRET=""

trap kotlin_cleanup EXIT INT TERM

# ──── Rust EchoService scaffold (mirrors swift-echo-app scaffold_service_guest) ────

scaffold_echo_service() {
    section "🧱 Scaffolding temporary Rust EchoService"
    run_actr init \
        -l rust --template echo --role service \
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
        awk '/^\[package\]/ {p=1; next} /^\[/ && p {exit} p && $1 == "version" {gsub(/"/, "", $3); print $3; exit}' \
            "$TMP_SERVICE_DIR/manifest.toml"
    )"
    [ -n "$SERVICE_VERSION" ] || fail "Unable to detect EchoService version"
    success "Rust EchoService ready: version ${SERVICE_VERSION}"
}

# Write the client manifest.toml — per actr-kt-migration §2.1 this file carries
# ONLY [package] (app ActrType) + [dependencies]. Connection config + ACL live in
# actr.toml (render_linked_client_config). [package].name is the app actor name so
# resolveManifestPackageActrType yields ${MANUFACTURER}:${APP_ACTOR}:${APP_VERSION}.
write_client_manifest() {
    section "📝 Writing Kotlin client manifest.toml"
    local acl_type="${MANUFACTURER}:${SERVICE_ACTOR}:${SERVICE_VERSION}"
    cat >"$TMP_APP_DIR/manifest.toml" <<EOF
edition = 1

[package]
name = "${APP_ACTOR}"
manufacturer = "${MANUFACTURER}"
version = "${APP_VERSION}"
description = "Actrium Kotlin EchoApp e2e client"

[dependencies]
${SERVICE_ACTOR} = { actr_type = "${acl_type}" }
EOF
    success "Client manifest.toml written (dep ${acl_type})"
}

# Place the remote EchoService proto locally + a minimal manifest.lock.toml placeholder.
# Per actr-kt-migration §2.3/§4 this is sufficient for `actr gen` — no registry /
# `actr deps install` needed.
seed_client_protos_and_lock() {
    section "📄 Seeding client protos + manifest.lock.toml"
    mkdir -p "$TMP_APP_DIR/protos/remote/echo-echo-server"
    cp "$REPO_ROOT/e2e/package-runtime-echo/proto/echo.proto" \
       "$TMP_APP_DIR/protos/remote/echo-echo-server/echo.proto"
    cat >"$TMP_APP_DIR/manifest.lock.toml" <<EOF
[metadata]
version = 1
generated_at = "2026-07-07T00:00:00+00:00"
EOF
    success "Remote echo.proto + placeholder manifest.lock.toml written"
}

# Drop the correct linked-mode instrumentation test into the scaffolded app,
# and replace the fixture's (non-compiling, pre-0.4) MainActivity with a minimal
# launcher stub — the instrumentation test drives the echo round-trip.
write_instrumentation_test() {
    section "✍️  Writing linked-mode instrumentation test + minimal MainActivity"
    local pkg_dir="$TMP_APP_DIR/app/src/androidTest/java/${APPLICATION_ID//.//}"
    local main_dir="$TMP_APP_DIR/app/src/main/java/${APPLICATION_ID//.//}"
    mkdir -p "$pkg_dir" "$main_dir"

    render_template \
        "$REPO_ROOT/e2e/kotlin-lib/EchoIntegrationTest.kt" \
        "$pkg_dir/EchoIntegrationTest.kt" \
        "__PACKAGE__=$APPLICATION_ID"
    rm -f "$pkg_dir/EchoIntegrationTest.kt.bak"

    render_template \
        "$REPO_ROOT/e2e/kotlin-lib/MainActivity.kt.tpl" \
        "$main_dir/MainActivity.kt" \
        "__PACKAGE__=$APPLICATION_ID"
    rm -f "$main_dir/MainActivity.kt.bak"

    success "Instrumentation test + minimal MainActivity written"
}

HOST_TARGET="$(rustc -vV | awk '/host:/ {print $2}')"

# ──── Main ────

section "🧪 Kotlin EchoApp E2E"
echo "Run directory:  $RUN_DIR"
echo "Service actor:  ${MANUFACTURER}:${SERVICE_ACTOR}"
echo "App actor:      ${MANUFACTURER}:${APP_ACTOR}:${APP_VERSION}"
echo "Actrix binary:  $ACTRIX_BIN"
echo "Host target:    $HOST_TARGET"

# Phase 0: native lib + local AAR (before any emulator dependence).
# Skip with SKIP_AAR_BUILD=1 when mavenLocal already has the AAR (local iteration
# or CI cache hit) to avoid the ~10 min native rebuild.
if [ "${SKIP_AAR_BUILD:-0}" != "1" ]; then
    build_and_publish_aar
else
    success "SKIP_AAR_BUILD=1 — assuming io.actrium:actr:${ACTR_KOTLIN_VERSION} is already in mavenLocal"
fi

# Phase 1: actrix infrastructure.
HOST_IP="$(detect_host_lan_ip)"
export HOST_IP
section "🌐 Host LAN IP for emulator connectivity: $HOST_IP"
render_runtime_configs
# The shared template's enable=25 = SIGNALING|AIS|SIGNER (STUN/TURN bits NOT set),
# so actrix logs "ICE服务(STUN/TURN)已禁用" and never starts the STUN/TURN server.
# The Android emulator needs STUN+TURN (direct ICE can't cross the QEMU NAT), so
# add ENABLE_STUN(2) + ENABLE_TURN(4) → enable=31.
perl -0pi -e 's/^enable = 25$/enable = 31/m' "$ACTRIX_CONFIG_PATH"
# Bind ICE (and HTTP) on all interfaces and advertise the host LAN IP so the
# Android emulator can reach STUN/TURN. Patterns are line-anchored (^) so the
# `ip = ` rewrite doesn't also clobber `advertised_ip`.
perl -0pi -e 's/^(ip) = "127\.0\.0\.1"/${1} = "0.0.0.0"/mg' "$ACTRIX_CONFIG_PATH"
perl -0pi -e "s/^advertised_ip = \"127\\.0\\.0\\.1\"/advertised_ip = \"${HOST_IP}\"/mg" "$ACTRIX_CONFIG_PATH"
rm -f "${ACTRIX_CONFIG_PATH}.bak"
build_local_actr_cli
build_local_actrix
start_actrix
login_admin
warmup_ais_key
ensure_realm
provision_mfr_keychain

# Phase 2: scaffold + publish the Rust EchoService + the client identity.
scaffold_echo_service
build_and_publish_service "$TMP_SERVICE_DIR" "$SERVICE_ACTOR" "$SERVICE_VERSION"
publish_app_package_identity "$APP_ACTOR" "$APP_VERSION"

# Phase 3: start the EchoService host and wait for registration.
run_server_host "$SERVICE_PACKAGE" "${MANUFACTURER}:${APP_ACTOR}:${APP_VERSION}" "kotlin-echo-server"
check_service_ready "$SERVICE_ACTOR"

# Phase 4: scaffold the Kotlin client, wire local AAR, seed protos+lock, gen.
# (No `actr deps install` — per actr-kt-migration §2.3/§4 a local proto + a
# placeholder manifest.lock.toml are sufficient for `actr gen`.)
scaffold_kotlin_app echo "$APP_ACTOR" "$TMP_APP_DIR" \
    "ws://10.0.2.2:${HTTP_PORT}/signaling/ws" "$MANUFACTURER"
write_client_manifest
seed_client_protos_and_lock
substitute_local_aar "$TMP_APP_DIR"
run_kotlin_gen "$TMP_APP_DIR" "$APPLICATION_ID"

# Phase 5: linked-mode client config + instrumentation test.
render_linked_client_config "$TMP_APP_DIR" "$APPLICATION_ID" \
    "${MANUFACTURER}:${SERVICE_ACTOR}:${SERVICE_VERSION}"
write_instrumentation_test

# Phase 6: run the instrumentation test on the emulator.
# Set ANDROID_SERIAL when multiple devices are attached (e.g. a phone + emulator).
run_connected_android_test "$TMP_APP_DIR" "$APPLICATION_ID"

echo ""
success "Kotlin EchoApp e2e completed successfully"
