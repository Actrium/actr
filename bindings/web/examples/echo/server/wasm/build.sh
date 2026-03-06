#!/bin/bash
set -e

echo "🔨 Building Echo Server WASM..."
echo "   User Workload + SW Runtime → 单个 WASM"

# Ensure a writable temp directory for wasm-bindgen install.
TMPDIR="${TMPDIR:-$(pwd)/.tmp}"
export TMPDIR
mkdir -p "$TMPDIR"

# 输出目录 (输出到 server/public)
OUT_DIR="../public"

# 清理旧的构建
rm -f "$OUT_DIR"/echo_server*.wasm "$OUT_DIR"/echo_server*.js "$OUT_DIR"/echo_server*.d.ts

# 构建 WASM
# - target: no-modules (适合 Service Worker，使用 wasm_bindgen 全局变量)
# - 包含: SW Runtime + User Workload (EchoService)
wasm-pack build \
  --target no-modules \
  --out-dir "$OUT_DIR" \
  --out-name echo_server \
  --release

# 清理不需要的文件
rm -f "$OUT_DIR/package.json" "$OUT_DIR/.gitignore"

echo ""
echo "✅ WASM built successfully!"
echo ""
echo "📁 Output: $OUT_DIR"
echo ""
echo "产物说明:"
echo "  - echo_server_bg.wasm  : WASM 主体（用户代码 + SW Runtime）"
echo "  - echo_server.js       : JS 胶水层（wasm-bindgen 生成）"
echo "  - echo_server.d.ts     : TypeScript 类型定义"
echo ""
echo "Files:"
ls -la "$OUT_DIR"/echo_server*

echo ""
echo "💡 下一步:"
echo "   1. 启动开发服务器: cd .. && pnpm dev"
