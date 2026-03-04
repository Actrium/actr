#!/bin/bash
set -e

echo "Building Service Worker Runtime..."

# Ensure a writable temp directory for wasm-bindgen install.
TMPDIR="${TMPDIR:-$(pwd)/.tmp}"
export TMPDIR
mkdir -p "$TMPDIR"

# 清理旧的构建
rm -rf ../../dist/sw

# 构建 WASM（target: no-modules，适合 Service Worker）
wasm-pack build \
  --target no-modules \
  --out-dir ../../dist/sw \
  --out-name actr_runtime_sw \
  --release

# 生成 npm 包信息
cat > ../../dist/sw/package.json << EOF
{
  "name": "@actor-rtc/runtime-sw",
  "version": "0.1.0",
  "description": "Actor-RTC Service Worker Runtime",
  "main": "actr_runtime_sw.js",
  "types": "actr_runtime_sw.d.ts",
  "files": [
    "actr_runtime_sw.wasm",
    "actr_runtime_sw.js",
    "actr_runtime_sw.d.ts"
  ]
}
EOF

echo "✓ Service Worker Runtime built successfully"
echo "  Output: dist/sw/"
ls -lh ../../dist/sw/
