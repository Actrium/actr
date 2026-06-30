# ACTR Kotlin

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

Kotlin/Android source bindings for the Actrium (ACTR) framework.

Official release artifacts are published from the package-sync repository:

- Repository: `Actrium/actr-kotlin-package-sync`
- Maven coordinate: `io.actrium:actr:<version>`
- Native libraries: [GitHub Release assets](https://github.com/Actrium/actr-kotlin-package-sync/releases)
  - `actr-kotlin-native.zip` — `jniLibs/` for arm64-v8a + x86_64

### Release flow

```
[actr monorepo release-train.sh]
  |
  | dispatch workflow_dispatch
  v
[actr-kotlin-package-sync/.github/workflows/release.yml]
  |
  | 1. Checkout Actrium/actr @ v{version}
  | 2. build-android.sh → jniLibs/{arm64-v8a,x86_64}/libactr.so
  | 3. scripts/package-binary.sh → dist/actr-kotlin-native.zip
  | 4. ./gradlew :actr-kotlin:publish → GitHub Packages (Maven)
  | 5. softprops/action-gh-release → GitHub Release asset
  v
[GitHub Release: actr-kotlin-package-sync/releases/tag/vX.Y.Z]
  + Maven artifact: io.actrium:actr:X.Y.Z @ GitHub Packages
```

Consumers add the Maven dependency:
```kotlin
repositories {
    maven {
        url = uri("https://maven.pkg.github.com/Actrium/actr-kotlin-package-sync")
        credentials {
            username = "<github-username>"
            password = "<github-token>"
        }
    }
}
dependencies {
    implementation("io.actrium:actr:<version>")
}
```

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

## Relationship to the Rust Node Typestate

The native host exposes a typestate chain
`Node<Init> → Node<Attached> → Node<Registered> → ActrRef`
(`from_config_file` → `attach_*` → `register` → `start`) so Rust-side
system code can hook into each transition. The Kotlin API collapses the
pipeline into a one-shot `ActrNode.fromPackageFile(...)` followed by
`start()`: Android/Kotlin app developers only see the node and the live
`ActrRef`. The `Node<S>` typestate is intentionally Rust-layer
power-user territory — bindings do not re-export it.

## Architecture

```
actr-kotlin/
├── actr-kotlin/              # Library module
│   └── src/main/kotlin/io/actrium/actr/
│       ├── actr.kt           # UniFFI-generated bindings (raw FFI layer)
│       └── dsl/              # High-level Kotlin-idiomatic API
│           ├── Actr.kt       # ActrNode/ActrRef wrapper classes + factory fns
│           ├── Types.kt      # Type builders (ActrType, ActrId, DataStream)
│           ├── Extensions.kt # Error handling, retry, context helpers
│           ├── RpcRequest.kt # Type-safe RPC protocol
│           ├── Workload.kt   # Workload abstractions (SimpleWorkload, etc.)
│           └── NetworkMonitor.kt  # Android network/lifecycle monitoring
├── demo/                     # Android demo app
└── scripts/                  # Build & packaging helpers
```

## Quick Start

### Prerequisites

- **Android Studio**: Arctic Fox or later
- **Android SDK**: API level 26+ (Android 8.0)
- **Rust**: 1.88+ with Android targets
- **protoc**: Protocol buffer compiler

### Build

```bash
# Build everything
./gradlew build

# Build library only
./gradlew :actr-kotlin:assembleRelease

# Build demo app
./gradlew :demo:assembleDebug
```

### Run Tests

```bash
# Unit tests
./gradlew test

# Android instrumentation tests (requires device/emulator)
./gradlew connectedDebugAndroidTest
```

## API Reference

Detailed API documentation: **[docs/api.md](docs/api.md)**

### Package-backed Node

```kotlin
import io.actrium.actr.dsl.*

// Create a node from config + package file
val node = ActrNode.fromPackageFile("config.toml", "dist/app.actr")

// Or with URL overloads
val node = ActrNode.fromPackageFile(configFileUrl, packageFileUrl)

// Start and get a running actor reference
val ref = node.start()

// RPC call with convenience defaults
val bytes = ref.call("echo.EchoService.Echo", requestPayload)

// Type-safe RPC with RpcRequest protocol
object EchoRpc : RpcRequest<EchoRequest, EchoResponse> {
    override val routeKey = "echo.EchoService.Echo"
    override fun serializeRequest(r: EchoRequest) = r.toByteArray()
    override fun deserializeResponse(b: ByteArray) = EchoResponse.parseFrom(b)
}
val response: EchoResponse = ref.call(EchoRpc, request)

// Discovery
val server = ref.discoverOne("acme:EchoService:1.0.0")

// Clean shutdown
ref.stop()
```

### Package-backed Runtime Observers

A package-backed node (`.actr` guest owns actor dispatch) can still observe
transport readiness for UI state and retry decisions. Build a `RuntimeObservers`
and pass it to any package-backed factory:

```kotlin
import io.actrium.actr.ContextBridge
import io.actrium.actr.WebRtcObserverBridge
import io.actrium.actr.dsl.*

val observers = runtimeObservers(
    webrtc = object : WebRtcObserverBridge {
        override suspend fun onConnecting(ctx: ContextBridge, event: PeerEvent) {
            // event.status == WebRtcPeerStatus.CONNECTING
        }
        override suspend fun onConnected(ctx: ContextBridge, event: PeerEvent) {
            // event.status == WebRtcPeerStatus.CONNECTED
        }
        override suspend fun onDisconnected(ctx: ContextBridge, event: PeerEvent) {
            // event.status == WebRtcPeerStatus.RECOVERING or WebRtcPeerStatus.IDLE
        }
    },
)

// observers is optional on every package-backed factory
val node = ActrNode.fromPackageFile("config.toml", "dist/app.actr", observers = observers)
// or with monitoring:
// val node = ActrNode.fromPackageFileWithMonitoring(..., observers = observers)
```

`PeerEvent.status` is a `WebRtcPeerStatus` (`CONNECTING`, `CONNECTED`, `RECOVERING`,
`IDLE`) for WebRTC peers and `null` for WebSocket peers, where send-readiness does
not apply. The `ActrNode`/`ActrRef` retain the `RuntimeObservers` as a
defense-in-depth measure mirroring `retainedWorkload` — UniFFI's callback handle
map is what actually keeps the host callbacks alive. See
[docs/api.md](docs/api.md#runtimeobservers-package-backed) for the full observer
surface (signaling, WebSocket, WebRTC, credential, mailbox).

### Linked (Kotlin-native) Workload

```kotlin
// Implement your workload
class MyWorkload : WorkloadLifecycleBridge {
    override suspend fun onStart(ctx: ContextBridge) { /* init */ }
    override suspend fun dispatch(ctx: ContextBridge, envelope: RpcEnvelopeBridge): ByteArray {
        // Handle incoming RPC
    }
    override suspend fun onStop(ctx: ContextBridge) { /* cleanup */ }
}

// Create and start
val workload = dynamicWorkload(MyWorkload())
val node = ActrNode.linked("config.toml", myActrType, workload)
val ref = node.start()

// Or with URL
val node = ActrNode.linked(configFileUrl, myActrType, workload)
```

### Network Monitoring (Android)

```kotlin
// Recommended: create a node that owns the NetworkEventHandle and monitor.
val node = ActrNode.fromPackageFileWithMonitoring(
    configPath = "config.toml",
    packagePath = "dist/app.actr",
    context = this,
    scope = lifecycleScope,
) { msg ->
    Log.d("App", msg)
}

override fun onResume() {
    super.onResume()
    node.onAppForeground()
}

override fun onPause() {
    node.onAppBackground()
    super.onPause()
}

// Manual monitor setup remains available for custom wiring.
var system: ActrNode? = null
val monitor = NetworkMonitor.create(this, lifecycleScope, { system }) { msg ->
    Log.d("App", msg)
}
monitor.startMonitoring()
```

### Error Handling & Retry

```kotlin
// Query error properties
when {
    ex.isTimeout -> println("Timed out")
    ex.isRecoverable -> println("Transient — retry")
    ex.requiresDlq -> println("Route to dead-letter queue")
}
println(ex.userMessage)

// Retry with exponential backoff
val result = withRetry(maxAttempts = 5) {
    ref.call("echo.EchoService.Echo", payload)
}

// Scoped actor lifecycle (auto-shutdown)
node.withStartedActor { ref ->
    val target = ref.discoverOne("acme:EchoService:1.0.0")
    val response = ref.call("echo.EchoService.Echo", payload)
}
```

### DSL Builders

```kotlin
// ActrType
val type = actrType("acme", "EchoService", "1.0.0")
val type = actrType { manufacturer = "acme"; name = "EchoService"; version = "1.0.0" }

// ActrId
val id = actrId { realm = 2281844430u; serialNumber = 1uL; type = "acme:EchoService:1.0.0" }

// DataStream
val stream = dataStream {
    streamId = "file-001"; sequence = 0uL; payload = data
    metadata { "content-type" to "application/octet-stream" }
}

// Workload
val wl = workload {
    realm = 2281844430u; type = "acme:my-service"
    onStart { ctx -> /* setup */ }
    onStop { ctx -> /* teardown */ }
}

// Manifest
val manifest = Manifest.from(Path.of("/app/actr.toml"))
val myType = manifest.packageType()
val aliases = manifest.dependencyAliases()
val echoType = manifest.resolveDependency("EchoService")
```

### Key Types

| Type | Description |
|------|-------------|
| `Manifest` | Parsed manifest.toml — typed access to package identity and dependency resolution |
| `ActrNode` | High-level node wrapper — creates and starts actors |
| `ActrRef` | Running actor reference — RPC, discovery, lifecycle |
| `ContextBridge` | Workload context — call/discover/send from within a workload |
| `RpcRequest<Req, Resp>` | Type-safe RPC contract (route + serialize/deserialize) |
| `DynamicWorkload` | Composite workload with lifecycle + optional observers |
| `NetworkEventHandle` | Platform network/lifecycle event callbacks |
| `PayloadType` | RPC/stream/media routing: RPC_RELIABLE, RPC_SIGNAL, STREAM_RELIABLE, etc. |
| `ActrException` | 11 error variants: Unavailable, TimedOut, NotFound, etc. |

## License

Licensed under the Apache License, Version 2.0.

## Related Projects

- [ACTR Framework](https://github.com/Actrium/actr) - Core Rust implementation
- [ACTR Examples](https://github.com/Actrium/actr/tree/main/examples) - Usage examples
