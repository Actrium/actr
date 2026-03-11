#!/usr/bin/env bash
# 重新编译 wasm_actor_fixture 并应用 asyncify 变换，更新 wasm_actor_fixture.rs
set -euo pipefail

WASM_OPT="${WASM_OPT:-/home/l/.cache/.wasm-pack/wasm-opt-1ceaaea8b7b5f7e0/bin/wasm-opt}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$SCRIPT_DIR/built"
BYTES_FILE="$SCRIPT_DIR/../wasm_actor_fixture.rs"

mkdir -p "$OUT_DIR"

echo "→ 编译 wasm-actor-fixture (wasm32-unknown-unknown) ..."
cd "$SCRIPT_DIR"
cargo build --release --target wasm32-unknown-unknown

RAW="$SCRIPT_DIR/target/wasm32-unknown-unknown/release/wasm_actor_fixture.wasm"

echo "→ 应用 wasm-opt --asyncify ..."
"$WASM_OPT" --asyncify -O "$RAW" -o "$OUT_DIR/wasm_actor_fixture.wasm"

echo "→ 生成 Rust bytes 文件 ..."
python3 - <<PYEOF
data = open('$OUT_DIR/wasm_actor_fixture.wasm', 'rb').read()
lines = ['pub const WASM_ACTOR_FIXTURE: &[u8] = &[']
for i in range(0, len(data), 16):
    chunk = data[i:i+16]
    lines.append('    ' + ', '.join(f'0x{b:02x}' for b in chunk) + ',')
lines.append('];')
open('$BYTES_FILE', 'w').write('\n'.join(lines))
print(f"写入 {len(data)} 字节 → $BYTES_FILE")
PYEOF

echo "✅ wasm_actor_fixture.rs 已更新"
