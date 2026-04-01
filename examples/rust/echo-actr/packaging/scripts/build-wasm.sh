#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
KEY_PATH="${ECHO_ACTR_SIGNING_KEY_PATH:-$ROOT_DIR/packaging/keys/dev-signing-key.json}"
VERSION="${ECHO_ACTR_VERSION:-$(python3 - <<'PY' "$ROOT_DIR/Cargo.toml"
import pathlib, sys, tomllib
data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
print(data["package"]["version"])
PY
)}"
ACTR_GIT_REV="${ECHO_ACTR_ACTR_GIT_REV:-$(python3 - <<'PY' "$ROOT_DIR/Cargo.toml"
import pathlib, sys, tomllib
data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
print(data["dependencies"]["actr-framework"]["rev"])
PY
)}"
ACTR_REPO_DIR="${ECHO_ACTR_ACTR_REPO_DIR:-$ROOT_DIR/../../..}"
ACTR_CLI_MANIFEST="${ECHO_ACTR_CLI_MANIFEST:-$ACTR_REPO_DIR/cli/Cargo.toml}"
TARGET="wasm32-unknown-unknown"
RAW_WASM="$ROOT_DIR/target/$TARGET/release/echo_guest.wasm"
FINAL_WASM="$DIST_DIR/echo-actr-${VERSION}-${TARGET}.wasm"
PACKAGE="$DIST_DIR/actrium-EchoService-${VERSION}-${TARGET}.actr"

ensure_actr_cli() {
  if [ -f "$ACTR_CLI_MANIFEST" ]; then
    ACTR_CLI_RUNNER=(cargo run --manifest-path "$ACTR_CLI_MANIFEST" --bin actr --)
    return 0
  fi

  if command -v actr >/dev/null 2>&1; then
    ACTR_CLI_RUNNER=(actr)
    return 0
  fi

  cargo install \
    --git https://github.com/Actrium/actr.git \
    --rev "$ACTR_GIT_REV" \
    --bin actr \
    actr-cli
  ACTR_CLI_RUNNER=("$HOME/.cargo/bin/actr")
}

mkdir -p "$DIST_DIR"
ensure_actr_cli

rustup target add "$TARGET" >/dev/null
cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --lib --release --target "$TARGET"

WASM_OPT="${WASM_OPT:-wasm-opt}"
"$WASM_OPT" --asyncify -O "$RAW_WASM" -o "$FINAL_WASM"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/echo-actr-package.XXXXXX")"
TMP_CONFIG="$TMP_DIR/package.toml"
trap 'rm -rf "$TMP_DIR"' EXIT

cat > "$TMP_CONFIG" <<EOF
[package]
manufacturer = "actrium"
name = "EchoService"
version = "$VERSION"
description = "Signed Echo guest actor distributed as an ActrPackage"
license = "Apache-2.0"
EOF

"${ACTR_CLI_RUNNER[@]}" pkg build \
  --binary "$FINAL_WASM" \
  --config "$TMP_CONFIG" \
  --key "$KEY_PATH" \
  --target "$TARGET" \
  --output "$PACKAGE"

echo "WASM artifacts written to $DIST_DIR"
