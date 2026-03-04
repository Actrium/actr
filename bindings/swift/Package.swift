// swift-tools-version: 6.0
import Foundation
import PackageDescription

// Binary distribution:
// - Default: fetch ActrFFI.xcframework from GitHub Release.
// - Local override: set ACTR_BINARY_PATH to a local xcframework path when developing.
let env = ProcessInfo.processInfo.environment
let bindingsPath = env["ACTR_BINDINGS_PATH"] ?? "ActrBindings"
let overrideBinaryPath = env["ACTR_BINARY_PATH"]

let releaseTag = env["ACTR_BINARY_TAG"] ?? "v0.1.29"
let remoteBinaryURL = "https://github.com/actor-rtc/actr-swift/releases/download/\(releaseTag)/ActrFFI.xcframework.zip"
let remoteBinaryChecksum = env["ACTR_BINARY_CHECKSUM"] ?? "403e8f520bf728edd4d01565e6ac72485c8adae9c4cb3fdbd7718c2a0af6137c"

let manifestDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path

func binaryPathRelativeToPackageRoot(_ path: String) -> String? {
    if path.hasPrefix("/") {
        let prefix = manifestDir.hasSuffix("/") ? manifestDir : "\(manifestDir)/"
        guard path.hasPrefix(prefix) else { return nil }
        return String(path.dropFirst(prefix.count))
    }
    return path
}

let actrBinaryTarget: Target
if let overrideBinaryPath {
    if let relativeBinaryPath = binaryPathRelativeToPackageRoot(overrideBinaryPath) {
        actrBinaryTarget = .binaryTarget(
            name: "ActrFFILib",
            path: relativeBinaryPath
        )
    } else {
        actrBinaryTarget = .binaryTarget(
            name: "ActrFFILib",
            url: remoteBinaryURL,
            checksum: remoteBinaryChecksum
        )
    }
} else {
    actrBinaryTarget = .binaryTarget(
        name: "ActrFFILib",
        url: remoteBinaryURL,
        checksum: remoteBinaryChecksum
    )
}

let package = Package(
    name: "actr-swift",
    platforms: [
        .iOS(.v15),
        .macOS(.v12),
    ],
    products: [
        .library(
            name: "Actr",
            targets: ["Actr"]
        ),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-protobuf.git", .upToNextMinor(from: "1.32.0")),
        .package(url: "https://github.com/actor-rtc/actr-protocols-swift.git", from: "0.1.2"),
    ],
    targets: [
        actrBinaryTarget,
        .target(
            name: "ActrFFI",
            path: bindingsPath,
            sources: ["actrFFI.c"],
            publicHeadersPath: "include"
        ),
        .target(
            name: "ActrBindings",
            dependencies: ["ActrFFI", "ActrFFILib"],
            path: bindingsPath,
            sources: ["Actr.swift"]
        ),
        .target(
            name: "Actr",
            dependencies: [
                "ActrFFI",
                "ActrBindings",
                "ActrFFILib",
                .product(name: "SwiftProtobuf", package: "swift-protobuf"),
                .product(name: "ActrProtocols", package: "actr-protocols-swift"),
            ]
        ),
    ]
)
