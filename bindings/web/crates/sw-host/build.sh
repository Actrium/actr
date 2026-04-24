#!/bin/bash
set -e

# Clear host-toolchain RUSTFLAGS so host-linker flags (e.g. `-fuse-ld=mold`
# from a global ~/.cargo/config.toml `[build] rustflags`) do not leak into
# the wasm32 target build. rust-lld does not recognize those linker args
# and errors out.
export RUSTFLAGS=""
export CARGO_ENCODED_RUSTFLAGS=""

echo "Building Service Worker Host..."

# Ensure a writable temp directory for wasm-bindgen install.
TMPDIR="${TMPDIR:-$(pwd)/.tmp}"
export TMPDIR
mkdir -p "$TMPDIR"

# Clean old builds
rm -rf ../../dist/sw

# Build WASM (target: no-modules, suitable for Service Workers)
wasm-pack build \
  --target no-modules \
  --out-dir ../../dist/sw \
  --out-name actr_sw_host \
  --release

# Generate npm package metadata
cat > ../../dist/sw/package.json << EOF
{
  "name": "@actor-rtc/sw-host",
  "version": "0.1.0",
  "description": "Actor-RTC Service Worker Host (Component Model bridge + runtime)",
  "main": "actr_sw_host.js",
  "types": "actr_sw_host.d.ts",
  "files": [
    "actr_sw_host.wasm",
    "actr_sw_host.js",
    "actr_sw_host.d.ts"
  ]
}
EOF

echo "✓ Service Worker Host built successfully"
echo "  Output: dist/sw/"
ls -lh ../../dist/sw/
