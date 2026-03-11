#!/usr/bin/env bash
set -euo pipefail

WASM_OPT="${WASM_OPT:-/home/l/.cache/.wasm-pack/wasm-opt-1ceaaea8b7b5f7e0/bin/wasm-opt}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$SCRIPT_DIR/built"

mkdir -p "$OUT_DIR"

echo "→ Compiling asyncify-fixture to wasm32..."
cd "$SCRIPT_DIR"
cargo build --release --target wasm32-unknown-unknown 2>&1

RAW="$SCRIPT_DIR/target/wasm32-unknown-unknown/release/asyncify_fixture.wasm"

echo "→ Applying asyncify transformation..."
"$WASM_OPT" --asyncify -O "$RAW" -o "$OUT_DIR/asyncify_fixture.wasm"

echo "→ Generating Rust bytes file..."
python3 - <<'PYEOF'
import os
data = open(os.environ.get('OUT_WASM', '/d/actor-rtc/actr/core/hyper/tests/asyncify_fixture/built/asyncify_fixture.wasm'), 'rb').read()
lines = ['pub const ASYNCIFY_FIXTURE_WASM: &[u8] = &[']
for i in range(0, len(data), 16):
    chunk = data[i:i+16]
    lines.append('    ' + ', '.join(f'0x{b:02x}' for b in chunk) + ',')
lines.append('];')
out = '\n'.join(lines)
outpath = '/d/actor-rtc/actr/core/hyper/tests/asyncify_fixture.rs'
open(outpath, 'w').write(out)
print(f"Written {len(data)} bytes → {outpath}")
PYEOF

echo "✅ Done. asyncify_fixture.rs ready."
