#!/bin/bash
set -e

# Clear host-toolchain RUSTFLAGS so host-linker flags (e.g. `-fuse-ld=mold`
# from a global ~/.cargo/config.toml `[build] rustflags`) do not leak into
# the wasm32 target build. rust-lld does not recognize those linker args
# and errors out.
export RUSTFLAGS=""
export CARGO_ENCODED_RUSTFLAGS=""

echo "Building DOM Bridge..."

# Clean old builds
rm -rf ../../dist/dom

# Build WASM (target: web, suitable for DOM)
wasm-pack build \
  --target web \
  --out-dir ../../dist/dom \
  --out-name actr_dom_bridge \
  --release

# Generate npm package metadata
cat > ../../dist/dom/package.json << EOF
{
  "name": "@actor-rtc/dom-bridge",
  "version": "0.1.0",
  "description": "Actor-RTC DOM Bridge (DOM-side bridge to the SW host)",
  "module": "actr_dom_bridge.js",
  "types": "actr_dom_bridge.d.ts",
  "sideEffects": [
    "actr_dom_bridge.js"
  ],
  "files": [
    "actr_dom_bridge.wasm",
    "actr_dom_bridge.js",
    "actr_dom_bridge.d.ts"
  ]
}
EOF

echo "✓ DOM Bridge built successfully"
echo "  Output: dist/dom/"
ls -lh ../../dist/dom/
