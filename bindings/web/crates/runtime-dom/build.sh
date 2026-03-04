#!/bin/bash
set -e

echo "Building DOM Runtime..."

# 清理旧的构建
rm -rf ../../dist/dom

# 构建 WASM（target: web，适合 DOM）
wasm-pack build \
  --target web \
  --out-dir ../../dist/dom \
  --out-name actr_runtime_dom \
  --release

# 生成 npm 包信息
cat > ../../dist/dom/package.json << EOF
{
  "name": "@actor-rtc/runtime-dom",
  "version": "0.1.0",
  "description": "Actor-RTC DOM Runtime",
  "module": "actr_runtime_dom.js",
  "types": "actr_runtime_dom.d.ts",
  "sideEffects": [
    "actr_runtime_dom.js"
  ],
  "files": [
    "actr_runtime_dom.wasm",
    "actr_runtime_dom.js",
    "actr_runtime_dom.d.ts"
  ]
}
EOF

echo "✓ DOM Runtime built successfully"
echo "  Output: dist/dom/"
ls -lh ../../dist/dom/
