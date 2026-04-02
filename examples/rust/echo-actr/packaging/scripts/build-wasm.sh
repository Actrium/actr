#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
KEY_PATH="${ECHO_ACTR_SIGNING_KEY_PATH:-$ROOT_DIR/packaging/keys/dev-signing-key.json}"
ACTR_GIT_REV="${ECHO_ACTR_ACTR_GIT_REV:-$(python3 - <<'PY' "$ROOT_DIR/Cargo.toml"
import pathlib, sys, tomllib
data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
print(data["dependencies"]["actr-framework"]["rev"])
PY
)}"
ACTR_REPO_DIR="${ECHO_ACTR_ACTR_REPO_DIR:-$ROOT_DIR/../../..}"
ACTR_CLI_MANIFEST="${ECHO_ACTR_CLI_MANIFEST:-$ACTR_REPO_DIR/cli/Cargo.toml}"
TARGET="wasm32-unknown-unknown"

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

ensure_actr_cli
"${ACTR_CLI_RUNNER[@]}" build \
  --file "$ROOT_DIR/manifest.toml" \
  --key "$KEY_PATH" \
  --target "$TARGET"
