#!/usr/bin/env bash
#
# kotlin-app.sh — shared helpers for the Kotlin Android e2e suites.
#
# The actrix/Rust lifecycle helpers are ported from e2e/swift-echo-app/run.sh and
# e2e/swift-datastream-app/run.sh; the Kotlin-specific helpers (AAR substitution,
# linked-mode app config, gradle connectedAndroidTest) are new.
#
# This file is SOURCED (after package-runtime-echo/lib/common.sh) by each
# e2e/kotlin-*/run.sh. It defines functions only; the run.sh owns globals and
# the main phase ordering.
#
# Globals expected from the run.sh:
#   REPO_ROOT, SCRIPT_DIR, RUN_DIR, SQLITE_DIR, LOG_DIR, DIST_DIR
#   HTTP_PORT, ICE_PORT, ADMIN_PASSWORD, MANUFACTURER, ADMIN_TOKEN
#   ACTR_CLI_MANIFEST, ACTR_TARGET_DIR, TEMP_SERVICE_TARGET_DIR
#   ACTRIX_CONFIG_TEMPLATE, ACTRIX_CONFIG_PATH, SERVER_RUNTIME_PATH
#   SERVICE_KEYCHAIN, SERVICE_PUBLIC_KEY, PROVISIONED_KEYCHAIN, PROVISIONED_PUBLIC_KEY
#   HOST_TARGET, ACTR_KOTLIN_VERSION (default "0.0.0-dev")

ACTR_KOTLIN_VERSION="${ACTR_KOTLIN_VERSION:-0.0.0-dev}"

# ──── actr CLI ────

run_actr() {
    CARGO_TARGET_DIR="$ACTR_TARGET_DIR" "$ACTR_CLI_BIN" "$@"
}

build_local_actr_cli() {
    section "🔧 Building local actr CLI"
    local features=("$@")
    local cargo_env=()

    # libssh2: use pkg-config when available (Linux apt, or brew on macOS).
    if command -v brew >/dev/null 2>&1; then
        local libssh2_prefix
        libssh2_prefix="$(brew --prefix libssh2 2>/dev/null || true)"
        if [ -n "$libssh2_prefix" ]; then
            cargo_env+=(
                "LIBSSH2_SYS_USE_PKG_CONFIG=1"
                "PKG_CONFIG_PATH=${libssh2_prefix}/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
                "CFLAGS=-I${libssh2_prefix}/include${CFLAGS:+ $CFLAGS}"
                "LDFLAGS=-L${libssh2_prefix}/lib${LDFLAGS:+ $LDFLAGS}"
            )
        fi
    fi
    if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists libssh2 2>/dev/null; then
        cargo_env+=(LIBSSH2_SYS_USE_PKG_CONFIG=1)
    fi

    env "${cargo_env[@]}" \
        CARGO_TARGET_DIR="$ACTR_TARGET_DIR" \
        cargo build --manifest-path "$ACTR_CLI_MANIFEST" --bin actr \
        ${features[@]+--features "${features[*]}"} >/dev/null
    ACTR_CLI_BIN="$ACTR_TARGET_DIR/debug/actr"
    [ -x "$ACTR_CLI_BIN" ] || fail "actr CLI binary missing at $ACTR_CLI_BIN"
    success "actr CLI ready: $ACTR_CLI_BIN"
}

# Build actrix from source into the e2e cache and export ACTRIX_BIN.
# Source dir defaults to the vendored $REPO_ROOT/actrix; override with
# ACTRIX_SOURCE_DIR (e.g. a sibling actrix checkout that has newer STUN/TURN
# fixes). Always building from source avoids picking up an unrelated `actrix`
# on PATH and guarantees control over which actrix is exercised.
build_local_actrix() {
    section "🔨 Building local actrix from source"
    local actrix_src="${ACTRIX_SOURCE_DIR:-$REPO_ROOT/actrix}"
    local actrix_manifest="$actrix_src/crates/actrixd/Cargo.toml"
    [ -f "$actrix_manifest" ] || fail "actrix manifest not found: $actrix_manifest"
    CARGO_TARGET_DIR="$ACTR_TARGET_DIR" cargo build --manifest-path "$actrix_manifest" --bin actrix >/dev/null
    ACTRIX_BIN="$ACTR_TARGET_DIR/debug/actrix"
    [ -x "$ACTRIX_BIN" ] || fail "actrix binary missing at $ACTRIX_BIN"
    export ACTRIX_BIN
    success "actrix ready (from $actrix_src): $ACTRIX_BIN"
}

# ──── Rust service packaging (ported from swift-echo-app) ────

append_workspace_patch() {
    local cargo_toml="$1"
    local repo_path="$REPO_ROOT"

    if ! grep -q '^\[workspace\]' "$cargo_toml"; then
        printf '\n[workspace]\n' >>"$cargo_toml"
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

pin_workspace_actr_dependencies() {
    local cargo_toml="$1"

    ACTR_FRAMEWORK_PATH="$REPO_ROOT/core/framework" \
    ACTR_PROTOCOL_PATH="$REPO_ROOT/core/protocol" \
        perl -i -pe '
            if (/^actr-framework = /) {
                $_ = qq{actr-framework = { path = "$ENV{ACTR_FRAMEWORK_PATH}" }\n};
            } elsif (/^actr-protocol = /) {
                $_ = qq{actr-protocol = { path = "$ENV{ACTR_PROTOCOL_PATH}" }\n};
            }
        ' "$cargo_toml"

    grep -Fq "actr-framework = { path = \"$REPO_ROOT/core/framework\" }" "$cargo_toml" ||
        fail "Failed to pin actr-framework to the workspace"
    grep -Fq "actr-protocol = { path = \"$REPO_ROOT/core/protocol\" }" "$cargo_toml" ||
        fail "Failed to pin actr-protocol to the workspace"
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

    curl -fsS -X POST \
        "http://127.0.0.1:${HTTP_PORT}/admin/api/mfr/apply" \
        -H "Authorization: Bearer ${ADMIN_TOKEN}" \
        -H 'Content-Type: application/json' \
        -d "{\"github_login\":\"${MANUFACTURER}\",\"contact\":\"e2e@local.actr\"}" \
        >"$apply_file"

    local mfr_id
    mfr_id="$(json_field "$apply_file" '.mfr_id')"

    curl -fsS -X POST \
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
        '{created_at:$created_at,note:"E2E manufacturer signing key issued by local actrix admin API",private_key:$private_key,public_key:$public_key}' \
        >"$PROVISIONED_KEYCHAIN"
    chmod 600 "$PROVISIONED_KEYCHAIN" 2>/dev/null || true

    jq -n --arg public_key "$(json_field "$approve_file" '.certificate.mfr_pubkey')" \
        '{public_key:$public_key}' >"$PROVISIONED_PUBLIC_KEY"

    success "Generated manufacturer keychain for ${MANUFACTURER}"
}

# Build + verify + publish a Rust service .actr package from an already-scaffolded
# service dir. Args: <service_dir> <service_actor_name> <service_version>.
build_and_publish_service() {
    local service_dir="$1"
    local service_actor="$2"
    local service_version="$3"
    section "📦 Building and publishing the ${service_actor} server package"

    SERVICE_PACKAGE="$DIST_DIR/${MANUFACTURER}-${service_actor}-${service_version}-${HOST_TARGET}.actr"
    (
        cd "$service_dir"
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
    success "${service_actor} package published"
}

# Publish a no-compile identity-marker package for a linked client actor type
# (actrix versions that require the actor type to be package-registered).
# Args: <app_actor_name> <app_version>.
publish_app_package_identity() {
    local app_actor="$1"
    local app_version="$2"
    section "📦 Publishing ${app_actor} package identity"

    local marker="$RUN_DIR/${app_actor}-identity.bin"
    local manifest="$RUN_DIR/${app_actor}-package-manifest.toml"
    local package="$DIST_DIR/${MANUFACTURER}-${app_actor}-${app_version}-${HOST_TARGET}.actr"
    printf 'linked %s identity marker\n' "$app_actor" >"$marker"
    cat >"$manifest" <<EOF
edition = 1

[package]
name = "${app_actor}"
manufacturer = "${MANUFACTURER}"
version = "${app_version}"
description = "Actrium ${app_actor} linked runtime identity marker"

[binary]
path = "${marker}"
target = "${HOST_TARGET}"
EOF

    run_actr build --no-compile \
        --manifest-path "$manifest" \
        --key "$PROVISIONED_KEYCHAIN" \
        --output "$package"
    run_actr pkg verify --pubkey "$PROVISIONED_PUBLIC_KEY" --package "$package" >/dev/null
    run_actr registry publish \
        --package "$package" \
        --keychain "$PROVISIONED_KEYCHAIN" \
        --endpoint "http://127.0.0.1:${HTTP_PORT}"
    success "${app_actor} package identity published"
}

# Write the package-backed server runtime config and launch `actr run` in the
# background. Args: <service_package> <client_acl_type> <observability_service_name>.
run_server_host() {
    local service_package="$1"
    local client_acl_type="$2"
    local obs_name="${3:-kotlin-e2e-server}"
    section "🚀 Starting package-backed server host"

    cat >"$SERVER_RUNTIME_PATH" <<EOF
edition = 1

[package]
path = "${service_package}"

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
tracing_service_name = "${obs_name}"

[webrtc]
force_relay = true
stun_urls = ["stun:${HOST_IP}:${ICE_PORT}"]
turn_urls = ["turn:${HOST_IP}:${ICE_PORT}"]

[acl]

[[acl.rules]]
permission = "allow"
type = "${client_acl_type}"
EOF

    # Launch in a backgrounded subshell that `exec`s the actr binary, so $! is the
    # actr process PID (not the subshell's). kotlin_cleanup then kills the real
    # server instead of orphaning it (the binary holds the signaling WebSocket +
    # bound ports). run_actr can't `exec` globally (it's also used for foreground
    # init/gen/build), so do the exec inline here.
    (
        export RUST_LOG="${RUST_LOG:-info}" CARGO_TARGET_DIR="$ACTR_TARGET_DIR"
        exec "$ACTR_CLI_BIN" run -c "$SERVER_RUNTIME_PATH"
    ) >"$LOG_DIR/server.log" 2>&1 &
    SERVER_PID=$!

    local attempt=0
    while [ $attempt -lt 30 ]; do
        if ! kill -0 "$SERVER_PID" 2>/dev/null; then
            cat "$LOG_DIR/server.log" >&2 || true
            fail "Server host exited early"
        fi
        if grep -q "fully started\|ActrNode started" "$LOG_DIR/server.log" 2>/dev/null; then
            success "Server host is running"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    warn "Server host readiness log not observed, continuing"
}

# Poll the signaling_cache.db until <service_actor> registers. Args: <service_actor>.
check_service_ready() {
    local service_actor="$1"
    section "🔍 Verifying ${service_actor} readiness"

    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        cat "$LOG_DIR/server.log" >&2 || true
        fail "${service_actor} process died before app launch"
    fi
    success "${service_actor} process alive (PID: $SERVER_PID)"

    curl -fsS "http://127.0.0.1:${HTTP_PORT}/signaling/health" >/dev/null 2>&1 \
        || fail "Signaling health check failed"
    success "Signaling health OK"

    local db_path="$SQLITE_DIR/signaling_cache.db"
    local timeout="${SERVICE_READY_TIMEOUT_SECONDS:-60}"
    if ! wait_for_service_registration \
        "$db_path" "$REALM_ID" "$MANUFACTURER" "$service_actor" "$timeout"; then
        echo "Service registrations observed before timeout:" >&2
        [ -f "$db_path" ] && sqlite3 "$db_path" \
            "SELECT actor_realm_id, actor_manufacturer, actor_device_name, service_name, status FROM service_registry;" \
            >&2 2>/dev/null || true
        tail -n 120 "$LOG_DIR/server.log" >&2 2>/dev/null || true
        fail "${service_actor} did not register with signaling within ${timeout}s"
    fi
    success "${service_actor} readiness check complete"
}

# ──── Kotlin app scaffolding ────

# Scaffold a Kotlin Android app via `actr init`. Args:
#   <template> <project_name> <out_dir> <signaling_url> <manufacturer>.
scaffold_kotlin_app() {
    local template="$1"
    local project_name="$2"
    local out_dir="$3"
    local signaling_url="$4"
    local manufacturer="$5"
    section "🧱 Scaffolding temporary Kotlin app (${template})"
    mkdir -p "$out_dir"
    (
        cd "$out_dir"
        run_actr init \
            -l kotlin \
            --template "$template" \
            --role app \
            --signaling "$signaling_url" \
            --manufacturer "$manufacturer" \
            --project-name "$project_name" .
    )
    success "Kotlin app scaffolded: $out_dir"
}

# Resolve the Android SDK root from the usual env vars / well-known locations.
resolve_android_sdk_root() {
    if [ -n "${ANDROID_SDK_ROOT:-}" ]; then printf '%s\n' "$ANDROID_SDK_ROOT"; return 0; fi
    if [ -n "${ANDROID_HOME:-}" ]; then printf '%s\n' "$ANDROID_HOME"; return 0; fi
    if [ -d "$HOME/Library/Android/sdk" ]; then printf '%s\n' "$HOME/Library/Android/sdk"; return 0; fi
    return 1
}

# Detect a host LAN IP that the Android emulator can route to (and host processes
# can also reach). The emulator can't use 127.0.0.1 (that's the emulator itself);
# 10.0.2.2 only maps to the host loopback and doesn't yield usable WebRTC
# candidates. A real LAN IP is reachable from both sides. Override with HOST_IP.
detect_host_lan_ip() {
    if [ -n "${HOST_IP:-}" ]; then printf '%s\n' "$HOST_IP"; return 0; fi
    local ip=""
    if command -v ipconfig >/dev/null 2>&1; then
        ip="$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || true)"
    fi
    if [ -z "$ip" ] && command -v hostname >/dev/null 2>&1; then
        ip="$(hostname -I 2>/dev/null | awk '{print $1}')"
    fi
    if [ -z "$ip" ] && command -v ip >/dev/null 2>&1; then
        ip="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src"){print $(i+1); exit}}')"
    fi
    [ -n "$ip" ] || fail "Could not detect a host LAN IP (set HOST_IP manually)"
    printf '%s\n' "$ip"
}

# Swap the scaffolded app to resolve the freshly-built local AAR from mavenLocal()
# instead of the published GitHub Packages artifact. Args: <app_dir>.
substitute_local_aar() {
    local app_dir="$1"
    section "🔌 Wiring local AAR (mavenLocal) into the scaffolded app"

    # 1. Replace settings.gradle.kts with a mavenLocal()-only variant (no GH auth).
    render_template \
        "$REPO_ROOT/e2e/kotlin-lib/settings.local.gradle.kts.tpl" \
        "$app_dir/settings.gradle.kts" \
        "__PROJECT_NAME__=$(basename "$app_dir")"

    # 2. Keep the canonical artifactId but replace the scaffold's fixed version
    #    with the version that was published to mavenLocal.
    perl -0pi -e "s/io\\.actrium:actr:[0-9][0-9a-z.\\-]*/io.actrium:actr:${ACTR_KOTLIN_VERSION}/g" \
        "$app_dir/app/build.gradle.kts"

    # 3. Write local.properties pointing at the Android SDK (gradle needs it).
    local sdk_root
    if sdk_root="$(resolve_android_sdk_root)"; then
        printf 'sdk.dir=%s\n' "$sdk_root" >"$app_dir/local.properties"
    else
        warn "ANDROID_SDK_ROOT not set; Gradle may fail to locate the SDK"
    fi
    success "Local AAR wired (io.actrium:actr:${ACTR_KOTLIN_VERSION} @ mavenLocal)"
}

# Build the actr-kotlin AAR for the emulator arch and publish it to mavenLocal().
# Uses ACTR_ANDROID_TARGETS (default: host arch — x86_64-linux-android on CI,
# aarch64-linux-android on Apple Silicon).
build_and_publish_aar() {
    section "🏗️  Building Android native lib + publishing actr-kotlin AAR"
    local sdk_root
    sdk_root="$(resolve_android_sdk_root)" || fail "Android SDK not found (set ANDROID_SDK_ROOT)"
    local kotlin_dir="$REPO_ROOT/bindings/kotlin"

    (
        cd "$kotlin_dir"
        ANDROID_SDK_ROOT="$sdk_root" \
        ACTR_COPY_DEMO_JNILIBS=false \
            bash build-android.sh
    )

    (
        cd "$kotlin_dir"
        ./gradlew :actr-kotlin:publishToMavenLocal --no-daemon -PactrVersion="$ACTR_KOTLIN_VERSION"
    )
    success "ACTR Kotlin AAR published to mavenLocal (io.actrium:actr:${ACTR_KOTLIN_VERSION})"
}

# Render the linked-mode actr.toml for the client into <app_dir>/app/src/androidTest/assets.
# Args: <app_dir> <application_id> <acl_type>.
render_linked_client_config() {
    local app_dir="$1"
    local application_id="$2"
    local acl_type="$3"
    local assets_dir="$app_dir/app/src/androidTest/assets"
    mkdir -p "$assets_dir"

    # All client endpoints use the host LAN IP ($HOST_IP) — reachable from the
    # Android emulator (which can't use 127.0.0.1) and from host processes alike.
    render_template \
        "$REPO_ROOT/e2e/kotlin-lib/actr.toml.tpl" \
        "$assets_dir/actr.toml" \
        "__HOST__=${HOST_IP}" \
        "__HTTP_PORT__=$HTTP_PORT" \
        "__ICE_PORT__=$ICE_PORT" \
        "__REALM_ID__=$REALM_ID" \
        "__REALM_SECRET__=$REALM_SECRET" \
        "__HYPER_DATA_DIR__=/data/data/${application_id}/files/hyper" \
        "__ACL_TYPE__=$acl_type"

    # manifest.toml is read by resolveActorType(); copy the (corrected) project manifest.
    cp "$app_dir/manifest.toml" "$assets_dir/manifest.toml"

    # manifest.lock.toml is required by the runtime; copy the one deps-install produced.
    if [ -f "$app_dir/manifest.lock.toml" ]; then
        cp "$app_dir/manifest.lock.toml" "$assets_dir/manifest.lock.toml"
    fi
    success "Linked client config written to $assets_dir"
}

# Run `actr gen -l kotlin` inside the app dir. Args: <app_dir> <application_id>.
# `-o app/src/main/java/<pkg>/generated` is required: the output path determines
# the generated package, so it must match the package `actr init` used for
# MainActivity (the application_id). Without -o, gen defaults to an `io.actr.*`
# prefix that doesn't match the `io.actrium.*` MainActivity and the build breaks.
run_kotlin_gen() {
    local app_dir="$1"
    local application_id="$2"
    local pkg_path="${application_id//.//}"
    section "⚙️  Generating Kotlin workload/dispatcher code"
    ( cd "$app_dir" && run_actr gen -l kotlin -c manifest.toml -i protos \
        -o "app/src/main/java/${pkg_path}/generated" )
    success "Kotlin code generated"
}

# Build both APKs, install them on the target device, and run the instrumentation
# test via `am instrument`. Targets ANDROID_SERIAL when set (required if more than
# one device is attached, e.g. a physical phone alongside the emulator); otherwise
# adb's single-device default is used. `connectedDebugAndroidTest` is avoided
# because it runs on ALL attached devices.
# Args: <app_dir> <application_id> [test_class_fqn].
run_connected_android_test() {
    local app_dir="$1"
    local app_id="$2"
    local test_class="${3:-${app_id}.EchoIntegrationTest}"
    local serial="${ANDROID_SERIAL:-}"
    local adb_flag=()
    [ -n "$serial" ] && adb_flag=(-s "$serial")

    section "🧪 Building app + androidTest APKs"
    ( cd "$app_dir" && ./gradlew assembleDebug assembleDebugAndroidTest --no-daemon )
    local app_apk="$app_dir/app/build/outputs/apk/debug/app-debug.apk"
    local test_apk="$app_dir/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"
    [ -f "$app_apk" ] || fail "app APK missing: $app_apk"
    [ -f "$test_apk" ] || fail "androidTest APK missing: $test_apk"

    section "📲 Installing APKs (${serial:-default device})"
    adb "${adb_flag[@]}" install -r "$app_apk" >/dev/null
    adb "${adb_flag[@]}" install -r "$test_apk" >/dev/null

    # The instrumentation target is <test applicationId>/<runner>. AGP defaults the
    # test app's applicationId to "<app.applicationId>.test"; the runner is set in
    # the fixture build.gradle.kts (testInstrumentationRunner).
    local instrument_target="${app_id}.test/androidx.test.runner.AndroidJUnitRunner"
    section "▶️  am instrument -e class ${test_class}"
    local instrument_log="${LOG_DIR:-/tmp}/instrument.log"
    # `am instrument` returns 0 even when tests fail (assertion or invalid-class
    # errors), so the pass/fail signal must come from the output, not the exit code.
    adb "${adb_flag[@]}" shell am instrument -w \
        -e class "${test_class}" \
        -e shortBackgroundSeconds "${SHORT_BACKGROUND_SECONDS:-5}" \
        -e longBackgroundSeconds "${LONG_BACKGROUND_SECONDS:-60}" \
        "$instrument_target" 2>&1 | tee "$instrument_log"

    local status=0
    if grep -qE "FAILURES!!!|Process crashed|java\.lang\.reflect\.|InvalidTestClassError" "$instrument_log"; then
        status=1
    elif ! grep -qE "OK \([0-9]+ test" "$instrument_log"; then
        # No explicit success marker and no known failure marker → treat as failure.
        status=1
    fi
    if [ $status -ne 0 ]; then
        warn "instrumentation FAILED; dumping adb logcat tail"
        adb "${adb_flag[@]}" logcat -d -t 500 2>/dev/null | tail -250 >&2 || true
    fi
    return $status
}

# ──── diagnostics + cleanup ────

capture_kotlin_diagnostics() {
    local diag_dir="$RUN_DIR/diagnostics"
    mkdir -p "$diag_dir"
    {
        echo "=== Process Status ==="
        echo "ACTRIX_PID=${ACTRIX_PID:-none} SERVER_PID=${SERVER_PID:-none}"
        [ -n "${ACTRIX_PID:-}" ] && kill -0 "$ACTRIX_PID" 2>/dev/null && echo "actrix: RUNNING" || echo "actrix: NOT RUNNING"
        [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null && echo "server: RUNNING" || echo "server: NOT RUNNING"
    } >"$diag_dir/process-status.txt" 2>/dev/null || true

    curl -fsS "http://127.0.0.1:${HTTP_PORT}/signaling/health" \
        >"$diag_dir/signaling-health.json" 2>/dev/null || true

    local db_path="$SQLITE_DIR/signaling_cache.db"
    if [ -f "$db_path" ]; then
        sqlite3 "$db_path" \
            "SELECT actor_realm_id, actor_manufacturer, actor_device_name, service_name, status FROM service_registry;" \
            >"$diag_dir/signaling-cache.txt" 2>/dev/null || true
    fi
    for log in "$LOG_DIR"/*.log; do
        [ -f "$log" ] || continue
        cp "$log" "$diag_dir/$(basename "$log")" 2>/dev/null || true
    done
    adb logcat -d 2>/dev/null >"$diag_dir/logcat.txt" || true
}

sanitize_and_stage_logs() {
    local sanitized="$RUN_DIR/sanitized-logs"
    local staged="$SCRIPT_DIR/.tmp/sanitized-logs"
    mkdir -p "$sanitized"
    local secrets=("$REALM_SECRET" "$ADMIN_PASSWORD" "${ADMIN_TOKEN:-}")
    local src="$RUN_DIR/diagnostics"
    [ -d "$src" ] || src="$LOG_DIR"
    while IFS= read -r -d '' f; do
        local rel="${f#$RUN_DIR/}"
        mkdir -p "$sanitized/$(dirname "$rel")"
        cp "$f" "$sanitized/$rel" 2>/dev/null || true
        case "$f" in
            *.log|*.txt|*.json)
                for s in "${secrets[@]}"; do
                    [ -n "$s" ] || continue
                    SECRET="$s" perl -0pi -e 's/\Q$ENV{SECRET}\E/REDACTED/g' "$sanitized/$rel" 2>/dev/null || true
                done
                ;;
        esac
    done < <(find "$src" -type f -print0 2>/dev/null)
    rm -rf "$staged"
    mkdir -p "$staged"
    cp -R "$sanitized/." "$staged/" 2>/dev/null || true
    echo "Sanitized logs staged at: $staged"
}

kotlin_cleanup() {
    local status=$?
    if [ $status -ne 0 ] || [ "${CAPTURE_DIAGNOSTICS_ON_SUCCESS:-0}" = "1" ]; then
        capture_kotlin_diagnostics || true
    fi
    sanitize_and_stage_logs || true
    for pid in "${SERVER_PID:-}" "${ACTRIX_PID:-}"; do
        [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    if [ $status -eq 0 ] && [ "${KEEP_TMP:-0}" != "1" ]; then
        rm -rf "$RUN_DIR"
    else
        echo "Artifacts preserved at: $RUN_DIR"
    fi
    exit $status
}
