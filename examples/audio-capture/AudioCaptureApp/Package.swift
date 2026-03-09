// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "AudioCaptureApp",
    platforms: [
        .macOS(.v14),
    ],
    dependencies: [
        .package(path: "../../../bindings/swift"),
    ],
    targets: [
        .executableTarget(
            name: "AudioCaptureApp",
            dependencies: [
                .product(name: "Actr", package: "swift"),
            ],
            path: "AudioCaptureApp",
            resources: [
                .process("actr.toml"),
            ]
        ),
    ]
)
