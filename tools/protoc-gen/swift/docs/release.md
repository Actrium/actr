# Release Guide

This project publishes a macOS arm64-only binary named `protoc-gen-actrframework-swift`.

## Versioning

- Tag format: `vX.Y.Z` (example: `v0.3.0`).
- Artifact name: `protoc-gen-actrframework-swift-macos-arm64.zip`.
- Checksum name: `protoc-gen-actrframework-swift-macos-arm64.zip.sha256`.

## Build locally

```bash
scripts/build-release.sh
```

Outputs:
- `dist/protoc-gen-actrframework-swift-macos-arm64.zip`
- `dist/protoc-gen-actrframework-swift-macos-arm64.zip.sha256`

## Publish

1. Create and push a tag:
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
2. GitHub Actions builds and uploads the release assets.
3. Update the Homebrew formula using the release URL and SHA256.

## Verification

1. Download the zip and checksum:
   ```bash
   shasum -a 256 -c protoc-gen-actrframework-swift-macos-arm64.zip.sha256
   unzip protoc-gen-actrframework-swift-macos-arm64.zip
   ```
2. Ensure the binary is on `PATH` and run `protoc`:
   ```bash
   cat <<'EOF' > example.proto
   syntax = "proto3";

   package demo;

   service EchoService {
     rpc Echo (EchoRequest) returns (EchoResponse);
   }

   message EchoRequest {
     string message = 1;
   }

   message EchoResponse {
     string reply = 1;
   }
   EOF

   protoc --actrframework-swift_out=. example.proto
   ```
3. Confirm `example.actor.swift` is generated.
