#!/usr/bin/env bash
# Rebuild wasm_actor_fixture, apply the asyncify transform, and update wasm_actor_fixture.rs.
set -euo pipefail

WASM_OPT="${WASM_OPT:-$(command -v wasm-opt || true)}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$SCRIPT_DIR/built"
BYTES_FILE="$SCRIPT_DIR/../wasm_actor_fixture.rs"

if [[ -z "$WASM_OPT" ]]; then
  echo "wasm-opt not found; set WASM_OPT or install wasm-opt" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

echo "→ Building wasm-actor-fixture (wasm32-unknown-unknown) ..."
cd "$SCRIPT_DIR"
cargo build --release --target wasm32-unknown-unknown

RAW="$SCRIPT_DIR/target/wasm32-unknown-unknown/release/wasm_actor_fixture.wasm"

echo "→ Applying wasm-opt --asyncify ..."
"$WASM_OPT" --asyncify -O "$RAW" -o "$OUT_DIR/wasm_actor_fixture.wasm"

echo "→ Generating Rust bytes file ..."
python3 - <<PYEOF
data = open('$OUT_DIR/wasm_actor_fixture.wasm', 'rb').read()
lines = ['pub const WASM_ACTOR_FIXTURE: &[u8] = &[']
for i in range(0, len(data), 16):
    chunk = data[i:i+16]
    lines.append('    ' + ', '.join(f'0x{b:02x}' for b in chunk) + ',')
lines.append('];')
open('$BYTES_FILE', 'w').write('\n'.join(lines))
print(f"Wrote {len(data)} bytes -> $BYTES_FILE")
PYEOF

echo "✅ wasm_actor_fixture.rs updated"
