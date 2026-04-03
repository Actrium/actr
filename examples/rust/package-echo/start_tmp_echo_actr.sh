#!/usr/bin/env bash
# Generate a temporary echo-actr service package via `actr init/install/gen`,
# then run the existing package-echo end-to-end flow against that workload.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ACTR_REPO_DIR="$(cd "$WORKSPACE_ROOT/../.." && pwd)"
LOG_DIR="$WORKSPACE_ROOT/logs"
DEFAULT_SIGNING_KEY="$WORKSPACE_ROOT/echo-actr/packaging/keys/dev-signing-key.json"
WORKSPACE_TARGET_DIR="$ACTR_REPO_DIR/target/examples"

mkdir -p "$LOG_DIR"

source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"

ORIGINAL_ARGS=("$@")
HAS_ARGS=$(( $# > 0 ? 1 : 0 ))
BACKEND="cdylib"
SEEN_BACKEND=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --backend)
            if [[ $# -lt 2 ]]; then
                echo "Missing value for --backend" >&2
                exit 1
            fi
            BACKEND="$2"
            SEEN_BACKEND=1
            shift 2
            ;;
        --backend=*)
            BACKEND="${1#--backend=}"
            SEEN_BACKEND=1
            shift
            ;;
        *)
            shift
            ;;
    esac
done

if [[ "$BACKEND" != "cdylib" ]]; then
    echo -e "${RED}❌ start_tmp_echo_actr.sh only supports --backend cdylib${NC}"
    echo "   Use ./start.sh for the fixed echo-actr wasm flow."
    exit 1
fi

if [[ $HAS_ARGS -eq 1 ]]; then
    START_ARGS=("${ORIGINAL_ARGS[@]}")
else
    START_ARGS=()
fi
if [[ $SEEN_BACKEND -eq 0 ]]; then
    START_ARGS=(--backend cdylib ${START_ARGS[@]+"${START_ARGS[@]}"})
fi

random_suffix() {
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -hex 1
        return
    fi
    printf '%02x\n' "$((RANDOM % 256))"
}

append_workspace_patch() {
    local cargo_toml="$1/Cargo.toml"
    if grep -q '^\[patch\.crates-io\]' "$cargo_toml"; then
        return
    fi

    cat >> "$cargo_toml" <<EOF

[patch.crates-io]
actr = { path = "$ACTR_REPO_DIR" }
actr-protocol = { path = "$ACTR_REPO_DIR/core/protocol" }
actr-framework = { path = "$ACTR_REPO_DIR/core/framework" }
actr-hyper = { path = "$ACTR_REPO_DIR/core/hyper" }
actr-runtime = { path = "$ACTR_REPO_DIR/core/runtime" }
actr-config = { path = "$ACTR_REPO_DIR/core/config" }
actr-service-compat = { path = "$ACTR_REPO_DIR/core/service-compat" }
actr-runtime-mailbox = { path = "$ACTR_REPO_DIR/core/runtime-mailbox" }
EOF
}

ensure_project_keychain_config() {
    local project_dir="$1"
    local signing_key="$2"
    local config_dir="$project_dir/.actr"
    local config_path="$config_dir/config.toml"

    mkdir -p "$config_dir"
    cat > "$config_path" <<EOF
[mfr]
keychain = "$signing_key"
EOF
}

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/echo-actr-verify.XXXXXX")"
TMP_ECHO_ACTR_DIR="$TMP_ROOT/echo-actr-$(random_suffix)"

cleanup_tmp() {
    local status=$?
    if [[ $status -eq 0 && "${KEEP_TMP_ECHO_ACTR:-0}" != "1" ]]; then
        rm -rf "$TMP_ROOT"
    else
        echo ""
        echo -e "${YELLOW}ℹ️  Temporary echo-actr preserved at:${NC} $TMP_ECHO_ACTR_DIR"
    fi
}
trap cleanup_tmp EXIT

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 Testing package-echo with a temporary echo-actr scaffold"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Temporary root: $TMP_ROOT"
echo "Project dir:    $TMP_ECHO_ACTR_DIR"

echo ""
echo -e "${BLUE}📦 Preparing codegen tools...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_cargo_bin "protoc-gen-prost" "protoc-gen-prost" "$LOG_DIR"

echo ""
echo -e "${BLUE}🔧 Building local actr CLI...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cargo build --manifest-path "$ACTR_REPO_DIR/cli/Cargo.toml" --bin actr >/dev/null
ACTR_CLI_BIN="$ACTR_REPO_DIR/target/debug/actr"
echo -e "${GREEN}✅ actr CLI ready: $ACTR_CLI_BIN${NC}"

if [[ ! -f "$DEFAULT_SIGNING_KEY" ]]; then
    echo -e "${RED}❌ signing key not found: $DEFAULT_SIGNING_KEY${NC}"
    exit 1
fi

echo ""
echo -e "${BLUE}🧱 Step 0: Scaffolding temporary echo-actr project...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
"$ACTR_CLI_BIN" init \
    -l rust \
    --template echo \
    --role service \
    --signaling ws://localhost:8081/signaling/ws \
    --manufacturer actrium \
    "$TMP_ECHO_ACTR_DIR"

if [[ -e "$TMP_ECHO_ACTR_DIR/src/generated" ]]; then
    echo -e "${RED}❌ init unexpectedly created src/generated${NC}"
    exit 1
fi
echo -e "${GREEN}✅ init left src/generated absent as expected${NC}"

append_workspace_patch "$TMP_ECHO_ACTR_DIR"

mkdir -p "$TMP_ECHO_ACTR_DIR/packaging/keys"
cp "$DEFAULT_SIGNING_KEY" "$TMP_ECHO_ACTR_DIR/packaging/keys/dev-signing-key.json"
jq '{public_key: .public_key}' \
    "$TMP_ECHO_ACTR_DIR/packaging/keys/dev-signing-key.json" \
    > "$TMP_ECHO_ACTR_DIR/public-key.json"
ensure_project_keychain_config \
    "$TMP_ECHO_ACTR_DIR" \
    "$TMP_ECHO_ACTR_DIR/packaging/keys/dev-signing-key.json"

echo ""
echo -e "${BLUE}⚙️  Step 1: Installing and generating workload code...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
(
    cd "$TMP_ECHO_ACTR_DIR"
    "$ACTR_CLI_BIN" install
    "$ACTR_CLI_BIN" gen -l rust
)

if [[ ! -f "$TMP_ECHO_ACTR_DIR/src/generated/actr-gen-meta.json" ]]; then
    echo -e "${RED}❌ generated metadata missing after actr gen${NC}"
    exit 1
fi
echo -e "${GREEN}✅ generated sources created via protoc-gen-actrframework${NC}"

echo ""
echo -e "${BLUE}🚀 Step 2: Running package-echo against the temporary workload...${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
CARGO_TARGET_DIR="$WORKSPACE_TARGET_DIR" \
ECHO_ACTR_DIR="$TMP_ECHO_ACTR_DIR" \
SERVER_ACTR_CONFIG="$SCRIPT_DIR/tmp_server-actr.toml" \
    "$SCRIPT_DIR/start.sh" ${START_ARGS[@]+"${START_ARGS[@]}"}
