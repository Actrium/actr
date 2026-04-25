#!/bin/bash
# Sync canonical web runtime artefacts into cli/assets/web-runtime/.
#
# `cli/src/web_assets.rs` embeds these files via `include_bytes!` /
# `include_str!`. Without this script, edits to the source-of-truth files
# silently drift from what the `actr` binary serves, leading to the kind of
# stale-asset confusion that produced the TD-005 false alarm and forced
# repeated manual copies during TD-006.
#
# Sources (canonical):
#   bindings/web/dist/sw/actr_sw_host_bg.wasm   ← from sw-host/build.sh
#   bindings/web/dist/sw/actr_sw_host.js        ← from sw-host/build.sh
#   bindings/web/packages/web-sdk/src/actor.sw.js
#   bindings/web/packages/web-sdk/src/actor-wbg.sw.js
#
# Destination:
#   cli/assets/web-runtime/<same names>
#
# Note: cli/assets/web-runtime/actr-host.html has no upstream source — it
# is edited in place — so this script does NOT touch it.
#
# Usage:
#   bash bindings/web/scripts/sync-cli-assets.sh           # sync only
#   bash bindings/web/scripts/sync-cli-assets.sh --build   # build sw-host wasm first
#   bash bindings/web/scripts/sync-cli-assets.sh --check   # verify in-sync, exit 1 on drift (CI use)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

DIST_SW="$REPO_ROOT/bindings/web/dist/sw"
WEB_SDK_SRC="$REPO_ROOT/bindings/web/packages/web-sdk/src"
CLI_ASSETS="$REPO_ROOT/cli/assets/web-runtime"

# (source, destination-name) pairs.
PAIRS=(
  "$DIST_SW/actr_sw_host_bg.wasm:actr_sw_host_bg.wasm"
  "$DIST_SW/actr_sw_host.js:actr_sw_host.js"
  "$WEB_SDK_SRC/actor.sw.js:actor.sw.js"
  "$WEB_SDK_SRC/actor-wbg.sw.js:actor-wbg.sw.js"
)

mode="sync"
for arg in "$@"; do
  case "$arg" in
    --build) mode="build-then-sync" ;;
    --check) mode="check" ;;
    -h|--help)
      sed -n '2,/^set /p' "$0" | sed 's/^# \?//;/^$/d'
      exit 0
      ;;
    *)
      echo "unknown arg: $arg" >&2
      exit 2
      ;;
  esac
done

if [ "$mode" = "build-then-sync" ]; then
  echo "→ rebuilding sw-host wasm via build.sh..."
  ( cd "$REPO_ROOT/bindings/web/crates/sw-host" && bash build.sh )
fi

mkdir -p "$CLI_ASSETS"

drift=0
for pair in "${PAIRS[@]}"; do
  src="${pair%%:*}"
  name="${pair##*:}"
  dst="$CLI_ASSETS/$name"

  if [ ! -f "$src" ]; then
    echo "✗ missing source: $src" >&2
    if [ "$mode" = "check" ]; then
      drift=1
      continue
    fi
    if [[ "$name" == actr_sw_host* ]]; then
      echo "  hint: run with --build, or first execute bindings/web/crates/sw-host/build.sh" >&2
    fi
    exit 1
  fi

  if [ "$mode" = "check" ]; then
    if [ ! -f "$dst" ] || ! cmp -s "$src" "$dst"; then
      echo "✗ drift: $name" >&2
      drift=1
    fi
  else
    if [ ! -f "$dst" ] || ! cmp -s "$src" "$dst"; then
      cp "$src" "$dst"
      echo "✓ synced: $name"
    else
      echo "= unchanged: $name"
    fi
  fi
done

if [ "$mode" = "check" ]; then
  if [ "$drift" -eq 0 ]; then
    echo "✓ cli/assets/web-runtime is in sync with bindings/web sources"
    exit 0
  else
    echo
    echo "cli/assets/web-runtime has drifted from bindings/web sources." >&2
    echo "Run: bash bindings/web/scripts/sync-cli-assets.sh" >&2
    exit 1
  fi
fi

echo
echo "Sync complete. Rebuild the actr binary so include_bytes! picks up the new content:"
echo "  cargo build -p actr-cli --bin actr"
