#!/bin/bash
# Build script for actr-kotlin Android native libraries
# This script builds the Rust library for Android targets and copies them to the demo app

set -e

# Get script directory (actr-kotlin root)
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
LIBACTR_DIR="$SCRIPT_DIR/libactr"

# Configuration
NDK_VERSION="25.2.9519653"
NDK_PATH="$HOME/Library/Android/sdk/ndk/$NDK_VERSION"
TOOLCHAIN_PATH="$NDK_PATH/toolchains/llvm/prebuilt/darwin-x86_64"
PROTOC_PATH="/opt/homebrew/bin/protoc"

# Output directories for jniLibs
# - Library module (for JitPack publishing)
LIBRARY_JNILIBS_DIR="$SCRIPT_DIR/actr-kotlin/src/main/jniLibs"
# - Demo app (for local development)
DEMO_JNILIBS_DIR="$SCRIPT_DIR/demo/src/main/jniLibs"

# Check if libactr submodule exists
if [ ! -d "$LIBACTR_DIR" ]; then
    echo "Error: libactr submodule not found at $LIBACTR_DIR"
    echo "Please run: git submodule update --init --recursive"
    exit 1
fi

# Check if NDK exists
if [ ! -d "$NDK_PATH" ]; then
    echo "Error: Android NDK not found at $NDK_PATH"
    echo "Please install NDK $NDK_VERSION via Android Studio SDK Manager"
    exit 1
fi

# Check if protoc exists
if [ ! -f "$PROTOC_PATH" ]; then
    echo "Error: protoc not found at $PROTOC_PATH"
    echo "Please install protobuf: brew install protobuf"
    exit 1
fi

# Change to libactr directory
cd "$LIBACTR_DIR"

# Set environment
export PROTOC="$PROTOC_PATH"

# Set up Android cross-compilation environment
export PATH="$TOOLCHAIN_PATH/bin:$PATH"

# aarch64-linux-android
export CC_aarch64_linux_android="$TOOLCHAIN_PATH/bin/aarch64-linux-android21-clang"
export AR_aarch64_linux_android="$TOOLCHAIN_PATH/bin/llvm-ar"
export RANLIB_aarch64_linux_android="$TOOLCHAIN_PATH/bin/llvm-ranlib"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOLCHAIN_PATH/bin/aarch64-linux-android21-clang"

# x86_64-linux-android
export CC_x86_64_linux_android="$TOOLCHAIN_PATH/bin/x86_64-linux-android21-clang"
export AR_x86_64_linux_android="$TOOLCHAIN_PATH/bin/llvm-ar"
export RANLIB_x86_64_linux_android="$TOOLCHAIN_PATH/bin/llvm-ranlib"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$TOOLCHAIN_PATH/bin/x86_64-linux-android21-clang"

echo "========================================"
echo "Building ACTR Android Native Libraries"
echo "========================================"
echo ""
echo "Source: $LIBACTR_DIR"
echo "Output (library): $LIBRARY_JNILIBS_DIR"
echo "Output (demo): $DEMO_JNILIBS_DIR"
echo ""

echo "Building for aarch64-linux-android..."
cargo build --release --target aarch64-linux-android

echo ""
echo "Building for x86_64-linux-android..."
cargo build --release --target x86_64-linux-android

# Create output directories
mkdir -p "$LIBRARY_JNILIBS_DIR/arm64-v8a" "$LIBRARY_JNILIBS_DIR/x86_64"
mkdir -p "$DEMO_JNILIBS_DIR/arm64-v8a" "$DEMO_JNILIBS_DIR/x86_64"

echo ""
echo "Copying native libraries..."
# Copy to library module (for JitPack publishing)
cp "$LIBACTR_DIR/target/aarch64-linux-android/release/libactr.so" "$LIBRARY_JNILIBS_DIR/arm64-v8a/"
cp "$LIBACTR_DIR/target/x86_64-linux-android/release/libactr.so" "$LIBRARY_JNILIBS_DIR/x86_64/"
# Copy to demo app (for local development)
cp "$LIBACTR_DIR/target/aarch64-linux-android/release/libactr.so" "$DEMO_JNILIBS_DIR/arm64-v8a/"
cp "$LIBACTR_DIR/target/x86_64-linux-android/release/libactr.so" "$DEMO_JNILIBS_DIR/x86_64/"

echo ""
echo "========================================"
echo "Build completed successfully!"
echo "========================================"
echo ""
echo "Library sizes (library module):"
ls -lh "$LIBRARY_JNILIBS_DIR"/*/*.so

echo ""
echo "Generating Kotlin bindings..."
echo ""

# Clean old generated files
echo "Cleaning old generated files..."
cd "$SCRIPT_DIR/actr-kotlin/src/main/kotlin/io/actor_rtc/actr"
rm -rf actr.kt io/

# Generate new Kotlin bindings
echo "Generating new Kotlin bindings..."
cd "$LIBACTR_DIR"
uniffi-bindgen generate --library target/debug/libactr.dylib --language kotlin --out-dir ../actr-kotlin/src/main/kotlin/io/actor_rtc/actr/

echo ""
echo "Kotlin bindings generated successfully!"
echo ""

echo "Next steps:"
echo "  1. Build the Android project: ./gradlew :demo:assembleDebug"
echo "  2. Run tests: ./gradlew :demo:connectedDebugAndroidTest"
echo ""
