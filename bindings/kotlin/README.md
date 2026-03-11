# ACTR Kotlin

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

Kotlin/Android source bindings for the Actor-RTC (ACTR) framework.

Official release artifacts are published from the package-sync repository:

- Repository: `Actrium/actr-kotlin-package-sync`
- Maven coordinate: `io.actrium:actr:<version>`

## Workspace Layout

The Kotlin build scripts build `libactr` from the monorepo workspace root.

```text
actr/
├── Cargo.toml                # Rust workspace root
├── bindings/
│   ├── ffi/                  # libactr crate
│   └── kotlin/               # Android module and build scripts
└── core/                     # Rust crates required by libactr
```

The package-sync repository owns its own workflow definitions.
The monorepo release train only dispatches that external release workflow with `version`, `source_sha`, and `source_tag`.

## 🎯 Overview

ACTR Kotlin provides seamless integration between the ACTR framework and Android/Kotlin applications. It enables developers to:

- Build distributed actor-based applications
- Leverage WebRTC for real-time communication
- Use type-safe Kotlin APIs with automatic code generation
- Integrate with existing Android applications

## 🏗️ Architecture

```
actr-kotlin/
├── actr-kotlin/              # 📚 ACTR Kotlin Library Module
│   ├── src/main/kotlin/io/actor_rtc/actr/
│   │   ├── ActrClient.kt     # Main client API
│   │   ├── Types.kt          # Core types (ActrId, ActrType, etc.)
│   │   └── generated/        # Auto-generated code from UniFFI
│   └── src/main/AndroidManifest.xml
├── demo/                     # 📱 Android Demo Application
│   ├── src/main/kotlin/com/example/actrdemo/
│   │   ├── MainActivity.kt   # Main app entry point
│   │   ├── ClientActivity.kt # Client demo
│   │   ├── ServerActivity.kt # Server demo
│   │   └── EchoIntegrationTest.kt # Integration tests
│   └── src/androidTest/      # Android instrumentation tests
├── proto/                    # 🔧 Protocol Buffer Definitions
│   └── local_file.proto      # File transfer service
├── build-android.sh          # 📦 Native library build script
└── build.gradle.kts          # Root build configuration
```

## 🔧 Key Technologies

- **UniFFI**: Type-safe Rust-to-Kotlin bindings
- **WebRTC**: Real-time communication protocol
- **Protocol Buffers**: Structured data serialization
- **Actor Model**: Distributed computing paradigm
- **Coroutines**: Asynchronous programming in Kotlin

## 🚀 Quick Start

### Prerequisites

- **Android Studio**: Arctic Fox or later
- **Android SDK**: API level 26+ (Android 8.0)
- **Rust**: 1.88+ with Android targets
- **protoc**: Protocol buffer compiler

### 1. Clone and Setup

```bash
git clone <repository-url>
cd actr/bindings/kotlin
```

### 2. Build Native Libraries

```bash
# Build Rust native libraries for Android (requires Android NDK)
./build-android.sh

# This will:
# - Build libactr for aarch64-linux-android (arm64-v8a)
# - Build libactr for x86_64-linux-android (x86_64)
# - Copy .so files to demo/src/main/jniLibs/
```

### 3. Build the Project

```bash
# Build everything
./gradlew build

# Build library only
./gradlew :actr-kotlin:assembleRelease

# Build demo app
./gradlew :demo:assembleDebug
```

### 4. Run Tests

```bash
# Run unit tests
./gradlew test

# Run Android instrumentation tests (requires device/emulator)
./gradlew connectedDebugAndroidTest
```

## 📖 Usage

### Basic Setup

```kotlin
import io.actorrtc.actr.*

// 1. Create configuration
val config = ActrConfig(
    signalingUrl = "ws://10.0.2.2:8081/signaling/ws", // For Android emulator
    actorType = ActrType("acme", "my.android.app"),
    realmId = 2281844430u
)

// 2. Initialize client
val client = ActrClient(config)

// 3. Connect to signaling server
val localActorId = client.connect()

// 4. Use the client for communication
// ... (see examples below)
```

### File Transfer Example

```kotlin
import com.example.LocalFileServiceWorkload
import com.example.MyLocalFileService
import local_file.File.*

// Create file service handler
val fileHandler = MyLocalFileService()
val workload = LocalFileServiceWorkload(fileHandler)

// Attach workload to client
val node = client.attach(workload)
val actorRef = node.start()

// Send file
val request = SendFileRequest.newBuilder()
    .setFilename("example.txt")
    .build()

val response = actorRef.call(
    targetId = actorRef.actorId(),
    method = "local_file.LocalFileService.SendFile",
    payloadType = PayloadType.RPC_RELIABLE,
    payload = request.toByteArray(),
    timeoutMs = 60000L
)

val sendResponse = SendFileResponse.parseFrom(response)
// Handle response...
```

### Service Discovery

```kotlin
// Discover available services
client.discoverRouteCandidates(
    targetType = ActrType("acme", "FileTransferService"),
    count = 5
) { result ->
    result.onSuccess { candidates ->
        if (candidates.isNotEmpty()) {
            val targetService = candidates.first()
            // Connect to discovered service
            performFileTransfer(targetService)
        }
    }
    result.onFailure { error ->
        Log.e(TAG, "Discovery failed: ${error.message}")
    }
}
```

## 🧪 Testing

### Key Test Cases

- **`testDataStreamToFileTransferReceiver`**: ✅ **PASSED**
  - Validates file transfer functionality
  - Tests data streaming capabilities
  - Confirms protobuf message handling

- **`testRpcCallToEchoServer`**: Requires external echo server
  - Tests RPC communication
  - Validates service discovery

### Running Tests

```bash
# Unit tests
./gradlew :actr-kotlin:test

# Integration tests (requires signaling server)
./gradlew :demo:connectedDebugAndroidTest
```

## 🔧 Development

### Code Generation

The project uses automatic code generation for:

1. **UniFFI Bindings**: Rust → Kotlin
2. **Protocol Buffers**: .proto → Kotlin/Java

### Building from Source

```bash
# 1. Build Rust library with Android targets and refresh UniFFI bindings
./build-android.sh

# 2. Build Android project
./gradlew :actr-kotlin:generateUniFFIBindings
./gradlew build
```

### Project Structure Details

- **`actr-kotlin/`**: Main library module
  - Contains UniFFI-generated bindings
  - Core ACTR types and APIs
  - Android-specific integrations

- **`demo/`**: Sample Android application
  - Demonstrates library usage
  - Contains integration tests
  - UI for testing features
  - **Network Monitoring**: Automatically monitors network state changes and calls `NetworkEventHandle` methods when connected

- **`proto/`**: Protocol definitions
  - Shared between Rust and Kotlin
  - Defines service interfaces
  - Message formats

## 📋 API Reference

### Core Classes

#### `ActrClient`
Main entry point for ACTR communication.

```kotlin
class ActrClient(config: ActrConfig) {
    fun connect(): ActrId
    fun disconnect()
    fun attach(workload: Workload): ActrNode
    fun discoverRouteCandidates(type: ActrType, count: Int): Result<List<ActrId>>
}
```

#### `ActrId`
Unique actor identifier.

```kotlin
data class ActrId(
    val actorType: ActrType,
    val serialNumber: Long,
    val realmId: UInt
) {
    fun toString(): String
}
```

#### `ActrType`
Actor type classification.

```kotlin
data class ActrType(
    val manufacturer: String,
    val name: String
) {
    fun toString(): String // Returns "manufacturer:name"
}
```

#### `NetworkEventHandle`
Handles network state changes for platform integration.

```kotlin
// Create network event handler
val networkHandle = system.createNetworkEventHandle()

// Handle network availability
val result = networkHandle.handleNetworkAvailableCatching()
result.onSuccess { eventResult ->
    println("Network became available")
}.onFailure { error ->
    println("Failed to handle network available: $error")
}

// Handle network loss
networkHandle.handleNetworkLostCatching().onSuccess {
    println("Network connection lost")
}

// Handle network type changes
networkHandle.handleNetworkTypeChangedCatching(isWifi = true, isCellular = false)
    .onSuccess { eventResult ->
        println("Network type changed to WiFi")
    }
```

**Note**: The demo application automatically monitors network state changes and calls these methods when connected to an ACTR system.

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Ensure `./gradlew build` passes
6. Submit a pull request

## 📄 License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

## 🔗 Related Projects

- [ACTR Framework](https://github.com/actor-rtc/actr) - Core Rust implementation
- [libactr](https://github.com/actor-rtc/libactr) - Rust FFI library (included as submodule)
- [ACTR Examples](https://github.com/actor-rtc/actr-examples) - Usage examples
- [ACTR CLI](https://github.com/actor-rtc/actr-cli) - Code generation tools

---

**Built with ❤️ by the Actor-RTC team**
