#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

ACTR_RELEASE_BIN="$REPO_ROOT/target/release/actr"
if [ -x "$ACTR_RELEASE_BIN" ]; then
  export ACTR_CMD="${ACTR_CMD:-$ACTR_RELEASE_BIN}"
  export ACTR_E2E_ACTR_BIN="${ACTR_E2E_ACTR_BIN:-$ACTR_RELEASE_BIN}"
fi

export SUITES="${SUITES:-BasicFunction}"

(
  cd "$REPO_ROOT/bindings/web/examples/echo"
  bash start-mock.sh
)

cd "$REPO_ROOT"
cargo test -p actr-cli --test e2e_typescript_generated_echo_web -- --ignored --test-threads=1
