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

TARGET="${1:-$(rustc -vV | awk '/host:/ {print $2}')}"

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

case "$TARGET" in
  aarch64-apple-darwin)
    LIB_PATH="$ROOT_DIR/target/$TARGET/release/libecho_guest.dylib"
    ;;
  aarch64-unknown-linux-gnu)
    LIB_PATH="$ROOT_DIR/target/$TARGET/release/libecho_guest.so"
    ;;
  x86_64-unknown-linux-gnu)
    LIB_PATH="$ROOT_DIR/target/$TARGET/release/libecho_guest.so"
    ;;
  x86_64-pc-windows-msvc)
    LIB_PATH="$ROOT_DIR/target/$TARGET/release/echo_guest.dll"
    ;;
  *)
    echo "unsupported native target: $TARGET" >&2
    exit 1
    ;;
esac

mkdir -p "$DIST_DIR"
ensure_actr_cli

cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --lib --release --target "$TARGET" --features cdylib

ARCHIVE="$DIST_DIR/echo-actr-${VERSION}-${TARGET}.zip"
PACKAGE="$DIST_DIR/actrium-EchoService-${VERSION}-${TARGET}.actr"

python3 - <<'PY' "$LIB_PATH" "$ARCHIVE"
import pathlib, sys, zipfile
lib_path = pathlib.Path(sys.argv[1])
archive = pathlib.Path(sys.argv[2])
with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zf:
    zf.write(lib_path, arcname=lib_path.name)
PY

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
  --binary "$LIB_PATH" \
  --config "$TMP_CONFIG" \
  --key "$KEY_PATH" \
  --target "$TARGET" \
  --output "$PACKAGE"

echo "Native artifacts written to $DIST_DIR"
