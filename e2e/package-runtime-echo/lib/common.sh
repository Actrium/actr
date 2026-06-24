#!/usr/bin/env bash

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

section() {
    echo ""
    echo -e "${BLUE}$1${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

success() {
    echo -e "${GREEN}✅ $1${NC}"
}

warn() {
    echo -e "${YELLOW}⚠️  $1${NC}"
}

fail() {
    echo -e "${RED}❌ $1${NC}" >&2
    exit 1
}

require_cmd() {
    local cmd="$1"
    if ! command -v "$cmd" >/dev/null 2>&1; then
        fail "Required command not found: $cmd"
    fi
}

require_executable() {
    local path_or_cmd="$1"
    if command -v "$path_or_cmd" >/dev/null 2>&1; then
        return 0
    fi
    if [ -x "$path_or_cmd" ]; then
        return 0
    fi
    fail "Executable not found: $path_or_cmd"
}

absolute_path() {
    local input="$1"
    if [ -d "$input" ]; then
        (cd "$input" && pwd)
        return 0
    fi

    local dir
    dir="$(dirname "$input")"
    local base
    base="$(basename "$input")"
    (cd "$dir" && printf '%s/%s\n' "$(pwd)" "$base")
}

cargo_bin_dir() {
    if [ -n "${CARGO_HOME:-}" ]; then
        printf '%s\n' "$CARGO_HOME/bin"
        return 0
    fi

    printf '%s\n' "$HOME/.cargo/bin"
}

prepend_path_once() {
    local dir="$1"
    case ":$PATH:" in
        *":$dir:"*) ;;
        *)
            PATH="$dir:$PATH"
            export PATH
            ;;
    esac
}

resolve_actrix_bin() {
    if [ -n "${ACTRIX_BIN:-}" ]; then
        if command -v "$ACTRIX_BIN" >/dev/null 2>&1; then
            command -v "$ACTRIX_BIN"
            return 0
        fi
        if [ -x "$ACTRIX_BIN" ]; then
            absolute_path "$ACTRIX_BIN"
            return 0
        fi
        fail "ACTRIX_BIN is set but not executable: $ACTRIX_BIN"
    fi

    if command -v actrix >/dev/null 2>&1; then
        command -v actrix
        return 0
    fi

    return 1
}

ensure_actrix_available() {
    local repo_root="$1"
    local actrix_repo_dir
    actrix_repo_dir="$repo_root/actrix"
    actrix_repo_dir="$(absolute_path "$actrix_repo_dir")"
    local actrix_crate_dir="$actrix_repo_dir/crates/actrixd"
    local actrix_manifest_path="$actrix_repo_dir/crates/actrixd/Cargo.toml"
    local cargo_bin
    cargo_bin="$(cargo_bin_dir)"
    local resolved=""

    if resolved="$(resolve_actrix_bin)"; then
        ACTRIX_BIN="$resolved"
        export ACTRIX_BIN
        return 0
    fi

    prepend_path_once "$cargo_bin"
    if resolved="$(resolve_actrix_bin)"; then
        ACTRIX_BIN="$resolved"
        export ACTRIX_BIN
        return 0
    fi

    require_cmd cargo
    [ -f "$actrix_manifest_path" ] || fail "Expected actrix crate manifest is missing: $actrix_manifest_path"

    section "🔨 Installing actrix into cargo user bin"
    cargo install --path "$actrix_crate_dir" --force

    prepend_path_once "$cargo_bin"
    if resolved="$(resolve_actrix_bin)"; then
        ACTRIX_BIN="$resolved"
        export ACTRIX_BIN
        success "actrix is ready: $ACTRIX_BIN"
        return 0
    fi

    local installed_bin="$cargo_bin/actrix"
    if [ -x "$installed_bin" ]; then
        ACTRIX_BIN="$(absolute_path "$installed_bin")"
        export ACTRIX_BIN
        success "actrix is ready: $ACTRIX_BIN"
        return 0
    fi

    fail "actrix installation completed but binary was not found in PATH or $cargo_bin"
}

kill_listener() {
    local protocol="$1"
    local port="$2"
    local pids=""

    case "$protocol" in
        tcp)
            pids="$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)"
            ;;
        udp)
            pids="$(lsof -tiUDP:"$port" 2>/dev/null || true)"
            ;;
        *)
            fail "Unsupported protocol for kill_listener: $protocol"
            ;;
    esac

    if [ -n "$pids" ]; then
        echo "Releasing ${protocol^^} port $port..."
        kill $pids 2>/dev/null || true
        sleep 1
    fi
}

wait_for_http_ok() {
    local url="$1"
    local timeout="$2"
    local started_at
    started_at="$(date +%s)"

    while true; do
        if curl -fsS "$url" >/dev/null 2>&1; then
            return 0
        fi

        local now
        now="$(date +%s)"
        if [ $((now - started_at)) -ge "$timeout" ]; then
            return 1
        fi
        sleep 1
    done
}

render_template() {
    local src="$1"
    local dst="$2"
    shift 2

    cp "$src" "$dst"
    while [ $# -gt 0 ]; do
        local key="${1%%=*}"
        local value="${1#*=}"
        local escaped
        escaped="$(printf '%s' "$value" | sed -e 's/[\\/&]/\\&/g')"
        sed -i.bak "s|$key|$escaped|g" "$dst"
        rm -f "$dst.bak"
        shift
    done
}

json_field() {
    local file="$1"
    local query="$2"
    jq -er "$query" "$file"
}

render_runtime_configs() {
    [ -n "${ACTRIX_CONFIG_TEMPLATE:-}" ] || fail "ACTRIX_CONFIG_TEMPLATE is not set"

    render_template \
        "$ACTRIX_CONFIG_TEMPLATE" \
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
    local realm_name="${REALM_NAME_PREFIX:-e2e}-${RUN_ID}"
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
