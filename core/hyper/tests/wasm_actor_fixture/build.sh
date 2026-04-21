#!/usr/bin/env bash
# Rebuild wasm_actor_fixture as a Component Model component and refresh
# the embedded-bytes file the integration tests consume.
#
# Phase 1 replaces the pre-existing core-wasm + wasm-opt asyncify transform
# with wasm-component-ld producing a real wasip2 Component. The linker
# bundled with Rust 1.91 is 0.5.17 which rejects the async custom sections
# wit-bindgen 0.57 emits; point RUSTFLAGS at 0.5.22+ (installable via
# `cargo install wasm-component-ld --version 0.5.22`).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$SCRIPT_DIR/built"
BYTES_FILE="$SCRIPT_DIR/../wasm_actor_fixture.rs"

LD="${WASM_COMPONENT_LD:-$HOME/.cargo/bin/wasm-component-ld}"
if [[ ! -x "$LD" ]]; then
    echo "wasm-component-ld not found at $LD" >&2
    echo "install with: cargo install wasm-component-ld --version 0.5.22" >&2
    exit 1
fi

mkdir -p "$OUT_DIR"

echo "-> Building wasm-actor-fixture (wasm32-wasip2) via wasm-component-ld $($LD --version)"
cd "$SCRIPT_DIR"
RUSTFLAGS="-Clinker=$LD" cargo build --release --target wasm32-wasip2

RAW="$SCRIPT_DIR/target/wasm32-wasip2/release/wasm_actor_fixture.wasm"
cp "$RAW" "$OUT_DIR/wasm_actor_fixture.wasm"

echo "-> Generating Rust bytes file"
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

echo "-> wasm_actor_fixture.rs updated"
