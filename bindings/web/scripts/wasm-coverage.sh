#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROBE_DIR="${ACTR_WEB_WASM_COVERAGE_DIR:-$ROOT_DIR/target/wasm-coverage}"
HTML_OUT="${ACTR_WEB_WASM_HTML_OUT:-$ROOT_DIR/target/wasm-coverage-html}"
LCOV_OUT="${ACTR_WEB_WASM_LCOV_OUT:-$ROOT_DIR/web-wasm-lcov.info}"

wasm_clang_works() {
  local compiler="$1"
  local out
  mkdir -p "$PROBE_DIR"
  out="$(mktemp "$PROBE_DIR/clang-probe.XXXXXX.o")"
  if "$compiler" --target=wasm32-unknown-unknown -x c -c /dev/null -o "$out" >/dev/null 2>&1; then
    rm -f "$out"
    return 0
  fi
  rm -f "$out"
  return 1
}

activate_wasm_cc() {
  local compiler="$1"
  local compiler_dir

  export CC_wasm32_unknown_unknown="$compiler"
  export TARGET_CC="$compiler"
  export CC="$compiler"

  compiler_dir="$(dirname "$compiler")"
  if [ "$compiler_dir" != "." ] && [ -d "$compiler_dir" ]; then
    export PATH="$compiler_dir:$PATH"
  fi
}

configure_wasm_cc() {
  if [ -n "${CC_wasm32_unknown_unknown:-}" ]; then
    wasm_clang_works "$CC_wasm32_unknown_unknown" || {
      echo "CC_wasm32_unknown_unknown does not support --target=wasm32-unknown-unknown: $CC_wasm32_unknown_unknown" >&2
      return 1
    }
    activate_wasm_cc "$CC_wasm32_unknown_unknown"
    return 0
  fi

  if [ -n "${TARGET_CC:-}" ]; then
    wasm_clang_works "$TARGET_CC" || {
      echo "TARGET_CC does not support --target=wasm32-unknown-unknown: $TARGET_CC" >&2
      return 1
    }
    activate_wasm_cc "$TARGET_CC"
    return 0
  fi

  local candidates=(
    /opt/homebrew/opt/llvm/bin/clang
    /usr/local/opt/llvm/bin/clang
    clang
  )

  local compiler
  for compiler in "${candidates[@]}"; do
    if command -v "$compiler" >/dev/null 2>&1 && wasm_clang_works "$compiler"; then
      activate_wasm_cc "$compiler"
      return 0
    fi
  done

  echo "No clang found that supports --target=wasm32-unknown-unknown." >&2
  echo "Install LLVM clang or set CC_wasm32_unknown_unknown to a wasm-capable clang." >&2
  return 1
}

configure_webdriver() {
  if [ "${WASM_BINDGEN_USE_BROWSER:-1}" != "1" ] || [ -n "${CHROMEDRIVER:-}" ]; then
    configure_webdriver_json
    return 0
  fi

  if command -v chromedriver >/dev/null 2>&1; then
    export CHROMEDRIVER="$(command -v chromedriver)"
    configure_webdriver_json
    return 0
  fi

  local driver
  driver="$(find "$ROOT_DIR/target/puppeteer-cache/chromedriver" -type f -name chromedriver -perm -111 2>/dev/null | sort | tail -n 1 || true)"
  if [ -n "$driver" ]; then
    export CHROMEDRIVER="$driver"
  fi

  configure_webdriver_json
}

configure_webdriver_json() {
  if [ -n "${WASM_BINDGEN_TEST_WEBDRIVER_JSON:-}" ]; then
    return 0
  fi

  local profile_dir="$PROBE_DIR/chrome-profile"
  local temp_dir="$PROBE_DIR/tmp"
  local webdriver_json="$PROBE_DIR/webdriver.json"
  mkdir -p "$profile_dir" "$temp_dir"
  export TMPDIR="$temp_dir"
  export TMP="$temp_dir"
  export TEMP="$temp_dir"
  printf '{"goog:chromeOptions":{"args":["user-data-dir=%s","disable-extensions"]}}\n' "$profile_dir" > "$webdriver_json"
  export WASM_BINDGEN_TEST_WEBDRIVER_JSON="$webdriver_json"
}

export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="${CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER:-wasm-bindgen-test-runner}"
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="${CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS:--Cinstrument-coverage -Zno-profiler-runtime -Clink-args=--no-gc-sections --cfg=wasm_bindgen_unstable_test_coverage}"
export WASM_BINDGEN_USE_BROWSER="${WASM_BINDGEN_USE_BROWSER:-1}"

packages=(
  -p actr-web-common
  -p actr-dom-bridge
)

cd "$ROOT_DIR"

configure_wasm_cc
configure_webdriver
rm -rf "$HTML_OUT"
mkdir -p "$HTML_OUT"

cargo +nightly llvm-cov clean --workspace

cargo +nightly llvm-cov test \
  --target wasm32-unknown-unknown \
  "${packages[@]}" \
  --all-features \
  --tests \
  --lcov \
  --output-path "$LCOV_OUT"

grep -q '^LF:' "$LCOV_OUT"

cargo +nightly llvm-cov report \
  --target wasm32-unknown-unknown \
  "${packages[@]}" \
  --html \
  --output-dir "$HTML_OUT"

if [ -s "$HTML_OUT/html/index.html" ] && [ ! -s "$HTML_OUT/index.html" ]; then
  cp -R "$HTML_OUT/html/." "$HTML_OUT/"
fi

test -s "$HTML_OUT/index.html"
echo "Wrote wasm LCOV to $LCOV_OUT"
echo "Wrote wasm HTML to $HTML_OUT"
