# framework-codegen-swift

Swift `protoc` plugin that generates ACTR framework service stubs.

Binary name: `protoc-gen-actrframework-swift`.

## Requirements

- macOS 13+ (arm64 only)
- Swift 6+ (for building from source)
- `protoc`

## Install (Homebrew)

```bash
brew tap actor-rtc/tap
brew install protoc-gen-actrframework-swift
```

## Install (Build from source)

Using Makefile:
```bash
make install
# Or install to a custom prefix
make install PREFIX=$HOME/.local
```

Manual:
```bash
swift build -c release --product protoc-gen-actrframework-swift --arch arm64
cp .build/arm64-apple-macosx/release/protoc-gen-actrframework-swift /usr/local/bin/
```

## Usage

Ensure `protoc-gen-actrframework-swift` is on your `PATH`, then run:

```bash
protoc --actrframework-swift_out=. path/to/your.proto
```

The plugin generates `*.actor.swift` (local) or `*.client.swift` (remote) files.

## Command Line Parameters

The plugin supports several parameters passed via `--actrframework-swift_out`:

| Parameter      | Default    | Description                                                  |
| -------------- | ---------- | ------------------------------------------------------------ |
| `ProtoSource`  | `Local`    | Global mode for all files (`Local` or `Remote`).             |
| `LocalFile`    |            | Specify a single `.proto` file as the local service handler. |
| `RemoteFiles`  |            | Colon-separated list of files to force as `Remote`.          |
| `Manufacturer` | `acme`     | Manufacturer name used for `ActrType` in remote forwarding.  |
| `Visibility`   | `Internal` | Access modifier for generated code (`Public` or `Internal`). |

### Smart Defaults
- If `LocalFile` is provided but `ProtoSource` is not, all other files default to `Remote`.
- When a file is `Local`, it generates a `Workload` actor that handles its own RPCs and **automatically proxies** all other `Remote` services.
- When a file is `Remote`, it only generates `RpcRequest` extensions for type-safe calling.

## Examples

### 1. Hybrid Node (One Local, One Remote)
The node implements `echo.proto` and proxies `chat.proto`:
```bash
protoc --actrframework-swift_out=LocalFile=echo.proto:. echo.proto chat.proto
```
Generates `echo.actor.swift` (with forwarding logic) and `chat.client.swift`.

### 2. Pure Remote Forwarder
A node that doesn't implement any local service but provides a package namespace:
```bash
protoc --actrframework-swift_out=LocalFile=client.proto:. client.proto chat.proto
```
If `client.proto` has no services, it generates `ClientWorkload` (or based on package name) which proxies `chat.proto`.

### 3. All Local (Default)
```bash
protoc --actrframework-swift_out=. echo.proto
```
Generates `echo.actor.swift` with standard handler protocol.

## Release

- Tag `vX.Y.Z` to trigger the GitHub Actions release workflow.
- Use `scripts/build-release.sh` for local packaging.
- Homebrew formula template: `homebrew/protoc-gen-actrframework-swift.rb`.
- Release checklist: `docs/release.md`.
