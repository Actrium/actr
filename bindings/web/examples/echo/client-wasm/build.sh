#!/bin/bash
set -e

echo "🔨 Building Echo Client WASM..."
echo "   Local Handler + SW Runtime → 单个 WASM"

# Ensure a writable temp directory for wasm-bindgen install.
TMPDIR="${TMPDIR:-$(pwd)/.tmp}"
export TMPDIR
mkdir -p "$TMPDIR"

# 输出目录 (输出到 client/public)
OUT_DIR="../client/public"

# 清理旧的构建
rm -f "$OUT_DIR"/echo_client*.wasm "$OUT_DIR"/echo_client*.js "$OUT_DIR"/echo_client*.d.ts
# 也清理旧的 bare runtime 文件
rm -f "$OUT_DIR"/actr_runtime_sw*.wasm "$OUT_DIR"/actr_runtime_sw*.js

# 构建 WASM
# - target: no-modules (适合 Service Worker，使用 wasm_bindgen 全局变量)
# - 包含: SW Runtime + Local Handler (EchoClient proxy)
wasm-pack build \
  --target no-modules \
  --out-dir "$OUT_DIR" \
  --out-name echo_client \
  --release

# 清理不需要的文件
rm -f "$OUT_DIR/package.json" "$OUT_DIR/.gitignore"

echo ""
echo "✅ WASM built successfully!"
echo ""
echo "📁 Output: $OUT_DIR"
echo ""
echo "产物说明:"
echo "  - echo_client_bg.wasm  : WASM 主体（Local Handler + SW Runtime）"
echo "  - echo_client.js       : JS 胶水层（wasm-bindgen 生成）"
echo "  - echo_client.d.ts     : TypeScript 类型定义"
echo ""
echo "Files:"
ls -la "$OUT_DIR"/echo_client*

echo ""
echo "💡 下一步:"
echo "   1. 启动开发服务器: cd ../client && pnpm dev"
