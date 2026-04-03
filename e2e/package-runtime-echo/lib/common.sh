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
    actrix_repo_dir="$repo_root/../actrix"
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
    if [ -d "$actrix_repo_dir" ]; then
        [ -f "$actrix_repo_dir/Cargo.toml" ] || fail "Found actrix source repo but Cargo.toml is missing: $actrix_repo_dir"
        section "🧱 Installing actrix from source repo"
    else
        require_cmd git
        section "📥 Cloning actrix source repo"
        git clone https://github.com/Actrium/actrix.git "$actrix_repo_dir"
    fi

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
