# Actr

Swift Package for distributing the ACTR (Actor-RTC) framework via a prebuilt XCFramework.

## Overview

- **ActrFFI.xcframework**: Precompiled iOS/macOS XCFramework published through GitHub Releases (remote `binaryTarget`).
- **Actr**: Swift API that includes UniFFI-generated bindings and high-level helpers.

## Consume via SwiftPM

```swift
dependencies: [
    .package(url: "https://github.com/actor-rtc/actr-swift.git", from: "0.1.0")
]
```

Targets that need the SDK should depend on `Actr`.

### Local development without a published binary

Set an environment override to point the package at a locally built XCFramework:

```bash
ACTR_BINARY_PATH=/absolute/path/to/ActrFFI.xcframework swift build
```

To build fresh bindings/binaries without dirtying the git worktree, point outputs to an ignored directory:

```bash
ACTR_BINDINGS_PATH=dist/ActrBindings ACTR_BINARY_PATH=dist/ActrFFI.xcframework ./build-xcframework.sh
```

Then consume the package with the same environment variables:

```bash
ACTR_BINDINGS_PATH=dist/ActrBindings ACTR_BINARY_PATH=dist/ActrFFI.xcframework swift build
```

## Build (maintainers)

Prerequisites:
- Rust 1.88+
- Xcode Command Line Tools
- UniFFI CLI: `cargo install uniffi --features "cli"`

Steps:

```bash
git submodule update --init --recursive
./build-xcframework.sh
```

This generates Swift bindings and the multi-platform XCFramework at `ActrFFI.xcframework/`.

## Package for release

1. Build the xcframework: `./build-xcframework.sh`
2. Package and compute checksum: `./scripts/package-binary.sh v0.1.0`
   - Outputs `dist/ActrFFI.xcframework.zip` and `dist/release.txt` with the checksum and URL.
3. Update `Package.swift` defaults:
   - Set `ACTR_BINARY_TAG` (default tag) and `ACTR_BINARY_CHECKSUM` (64-hex checksum) to match `dist/release.txt`.
4. Push code/tag to GitHub `actor-rtc/actr-swift` and create a Release with the zip asset:
   - `gh release create v0.1.0 dist/ActrFFI.xcframework.zip --notes "ActrFFI v0.1.0"`
5. Consumers can then resolve the package without building Rust locally.

## Project Structure

- `libactr/`: Rust library (submodule)
- `ActrBindings/`: UniFFI-generated Swift bindings (Swift + headers/modulemap)
- `build-xcframework.sh`: Build script
- `scripts/package-binary.sh`: Zip + checksum helper for Release assets
- `dist/`: Local release artifacts (ignored)
- `Package.swift`: Swift Package manifest

## Configuration

UniFFI configuration lives in `libactr/uniffi.toml`.

## Documentation

- **[API Reference](docs/api.md)** - Comprehensive API documentation covering both Low Level and High Level APIs

## License

Apache-2.0
