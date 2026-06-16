# Actr Kotlin API Reference

This document provides a comprehensive reference for both the low-level (UniFFI-generated) and high-level (DSL) APIs.

## Package Structure

| Package | Purpose |
|---------|---------|
| `io.actrium.actr` | UniFFI-generated raw bindings (import for low-level access) |
| `io.actrium.actr.dsl` | High-level Kotlin-idiomatic API (recommended for app developers) |

For most use cases, `import io.actrium.actr.dsl.*` gives you everything you need.

---

## High-Level API (DSL Layer)

### ActrNode

High-level wrapper around the generated `ActrNode`. Retains `DynamicWorkload` references to prevent premature garbage collection.

**Companion factories:**

| Method | Description |
|--------|-------------|
| `suspend fun fromPackageFile(configPath: String, packagePath: String): ActrNode` | Create from TOML config + `.actr` package file paths |
| `suspend fun fromPackageFile(configURL: URL, packageURL: URL): ActrNode` | URL-based overload — validates file URLs |
| `suspend fun linked(configPath: String, actorType: ActrType, workload: DynamicWorkload): ActrNode` | Create a linked node with a Kotlin workload |
| `suspend fun linked(configURL: URL, actorType: ActrType, workload: DynamicWorkload): ActrNode` | URL-based linked overload |

**Instance methods:**

| Method | Description |
|--------|-------------|
| `suspend fun start(): ActrRef` | Start the actor, returns a running reference retaining the workload |
| `suspend fun createNetworkEventHandle(): NetworkEventHandle` | Create a handle for platform network event callbacks |
| `suspend fun <T> withStartedActor(block: suspend (ActrRef) -> T): T` | Execute a block with auto-shutdown on exit |
| `fun close()` | Release native resources (implements `AutoCloseable`) |

**Top-level convenience functions:**

| Function | Description |
|----------|-------------|
| `suspend fun createActrNode(configPath, packagePath): ActrNode` | Alias for `ActrNode.fromPackageFile` |
| `suspend fun linked(configPath, actorType, workload): ActrNode` | Alias for `ActrNode.linked` |

---

### ActrRef

High-level wrapper around the generated `ActrRefWrapper`. Retains `DynamicWorkload` to prevent GC.

| Method | Description |
|--------|-------------|
| `fun actorId(): ActrId` | Get the actor's unique identifier |
| `suspend fun call(routeKey: String, payloadType: PayloadType, requestPayload: ByteArray, timeoutMs: Long): ByteArray` | Raw RPC call with full parameters |
| `suspend fun call(routeKey: String, requestPayload: ByteArray, payloadType = RPC_RELIABLE, timeoutMs = 30000L): ByteArray` | **Extension** — RPC with convenient parameter order and defaults |
| `suspend fun <Req, Resp> call(rpc: RpcRequest<Req, Resp>, request: Req, ...): Resp` | **Extension** — Type-safe RPC via `RpcRequest` contract |
| `suspend inline fun <Req, Resp> call(routeKey, request, serialize, deserialize, ...): Resp` | **Extension** — Inline type-safe RPC with lambdas |
| `suspend fun tell(routeKey: String, payloadType: PayloadType, messagePayload: ByteArray)` | Send a one-way message (fire-and-forget) |
| `suspend fun tell(routeKey: String, messagePayload: ByteArray, payloadType = RPC_RELIABLE)` | **Extension** — Tell with convenient parameter order and defaults |
| `suspend fun discover(targetType: ActrType, count: UInt): List<ActrId>` | Discover actors by type |
| `suspend fun discover(typeString: String, count: UInt = 1u): List<ActrId>` | **Extension** — Discover by type string |
| `suspend fun discoverOne(typeString: String): ActrId?` | **Extension** — Discover single by type string |
| `suspend fun discoverOne(type: ActrType): ActrId?` | **Extension** — Discover single by ActrType |
| `fun isShuttingDown(): Boolean` | Check if actor is shutting down |
| `val isActive: Boolean` | Whether the actor reference is still valid |
| `fun shutdown()` | Trigger shutdown |
| `suspend fun waitForShutdown()` | Wait for shutdown to complete |
| `suspend fun stop()` | Shutdown + wait (recommended) |
| `suspend fun awaitShutdown()` | **Extension** — Alias for `waitForShutdown()` |
| `fun close()` | Release native resources |
| `suspend fun callCatching(...): Result<ByteArray>` | **Extension** — Result-wrapped RPC call |
| `suspend fun discoverCatching(typeString, count): Result<List<ActrId>>` | **Extension** — Result-wrapped discovery |

---

### RpcRequest<Req, Resp>

Type-safe RPC contract interface. Implement once per RPC method for compile-time type safety.

```kotlin
interface RpcRequest<Req, Resp> {
    val routeKey: String
    fun serializeRequest(request: Req): ByteArray
    fun deserializeResponse(bytes: ByteArray): Resp
}
```

**Extension functions:**

```kotlin
// Interface-based call
suspend fun <Req, Resp> ActrRef.call(
    rpc: RpcRequest<Req, Resp>,
    request: Req,
    payloadType: PayloadType = RPC_RELIABLE,
    timeoutMs: Long = 30000L,
): Resp

// Inline lambda-based call (no RpcRequest object needed)
suspend inline fun <Req, Resp> ActrRef.call(
    routeKey: String,
    request: Req,
    payloadType: PayloadType = RPC_RELIABLE,
    timeoutMs: Long = 30000L,
    crossinline serialize: (Req) -> ByteArray,
    crossinline deserialize: (ByteArray) -> Resp,
): Resp
```

---

### ContextBridge

Context passed to workload callbacks. Provides methods for inter-actor communication from within a workload.

**Generated methods (from UniFFI):**

| Method | Description |
|--------|-------------|
| `suspend fun callRaw(target, routeKey, payloadType, payload, timeoutMs): ByteArray` | Raw RPC call |
| `suspend fun tellRaw(target, routeKey, payloadType, payload)` | Fire-and-forget message |
| `suspend fun discover(targetType: ActrType): ActrId` | Discover remote actors |
| `suspend fun sendDataStream(target, chunk, payloadType)` | Send a data stream chunk |
| `suspend fun registerStream(streamId, callback)` | Register a data stream callback |
| `suspend fun unregisterStream(streamId)` | Unregister a data stream |
| `suspend fun registerMediaTrack(trackId, callback)` | Register a media track callback |
| `suspend fun unregisterMediaTrack(trackId)` | Unregister a media track |

**DSL convenience extensions:**

```kotlin
// Convenience call with defaults (RPC_RELIABLE, 30s timeout)
suspend fun ContextBridge.call(
    target: ActrId,
    routeKey: String,
    payload: ByteArray,
    payloadType: PayloadType = RPC_RELIABLE,
    timeoutMs: Long = 30000L,
): ByteArray
```

---

### Workload

Type alias: `typealias Workload = WorkloadLifecycleBridge`

Core callback interface for Kotlin-native workloads:

```kotlin
interface WorkloadLifecycleBridge {
    suspend fun onStart(ctx: ContextBridge)
    suspend fun onReady(ctx: ContextBridge)
    suspend fun onStop(ctx: ContextBridge)
    suspend fun onError(ctx: ContextBridge, event: ErrorEventBridge)
    suspend fun dispatch(ctx: ContextBridge, envelope: RpcEnvelopeBridge): ByteArray
}
```

**Workload abstractions:**

| Class | Description |
|-------|-------------|
| `SimpleWorkload` | Concrete workload with DataStream channel support, target server routing, and handler hooks |
| `RoutedWorkload` | Abstract base class with target server routing — subclass and override lifecycle hooks |
| `WorkloadBuilder` | DSL builder: `workload { realm = ...; type = ...; onStart { }; onStop { } }` |

**Factory functions:**

```kotlin
// DSL builder
inline fun workload(builder: WorkloadBuilder.() -> Unit): SimpleWorkload

// Composite workload with optional observers
fun dynamicWorkload(
    lifecycle: WorkloadLifecycleBridge,
    signaling: SignalingObserverBridge? = null,
    websocket: WebSocketObserverBridge? = null,
    webrtc: WebRtcObserverBridge? = null,
    credential: CredentialObserverBridge? = null,
    mailbox: MailboxObserverBridge? = null,
): DynamicWorkload
```

- `DynamicWorkload` typealias for `io.actrium.actr.DynamicWorkload`

---

### Types

#### ActrType

Actor type identifier (manufacturer:name:version).

```kotlin
// Factory functions
fun actrType(manufacturer: String, name: String, version: String): ActrType
inline fun actrType(builder: ActrTypeBuilder.() -> Unit): ActrType
fun String.toActrType(): ActrType

// Extensions
fun ActrType.toTypeString(): String          // "manufacturer:name:version"
fun ActrType.matches(typeString: String): Boolean
```

#### ActrId

Actor identifier (realm + serial number + type).

```kotlin
// Factory
inline fun actrId(builder: ActrIdBuilder.() -> Unit): ActrId

// Extensions
val ActrId.realmId: UInt
fun ActrId.toShortString(): String
fun ActrId.toFullString(): String
```

#### DataStream

Streaming data chunk with metadata.

```kotlin
// Factory
inline fun dataStream(builder: DataStreamBuilder.() -> Unit): DataStream

// Extensions
fun DataStream.getMetadata(key: String): String?
fun DataStream.hasMetadata(key: String): Boolean
fun DataStream.metadataMap(): Map<String, String>
```

#### Realm

Security realm identifier.

```kotlin
fun realm(id: UInt): Realm
fun realm(id: Int): Realm
```

---

### Manifest

Typed access to `actr.toml` manifest files — resolve package identity and dependency
types without hardcoding `"manufacturer:name:version"` strings.

#### Manifest class

The recommended Kotlin entry point. Construct with a `Path`, `File`, or raw path
string, then query package type, dependency aliases, and resolved dependency types.

```kotlin
class Manifest(manifestPath: String) {
    fun packageType(): ActrType
    fun resolveDependency(alias: String): ActrType
    fun dependencyAliases(): List<String>

    companion object {
        fun from(path: Path): Manifest
        fun from(file: File): Manifest
    }
}
```

**Example:**

```kotlin
val manifest = Manifest.from(Path.of("/app/actr.toml"))
val myType = manifest.packageType()          // ActrType of [package]
val aliases = manifest.dependencyAliases()   // List of all dependency aliases
val echoType = manifest.resolveDependency("EchoService")  // Resolved ActrType
```

#### Top-level functions

Path/File overloads of the raw UniFFI bindings:

```kotlin
// Package type
fun resolveManifestPackageActrType(manifestPath: String): ActrType
fun resolveManifestPackageActrType(manifestPath: Path): ActrType
fun resolveManifestPackageActrType(manifestFile: File): ActrType

// Dependency resolution
fun resolveManifestDependency(manifestPath: String, dependencyAlias: String): ActrType
fun resolveManifestDependency(manifestPath: Path, dependencyAlias: String): ActrType
fun resolveManifestDependency(manifestFile: File, dependencyAlias: String): ActrType

// Alias list
fun resolveManifestDependencyAliasList(manifestPath: String): List<String>
fun resolveManifestDependencyAliasList(manifestPath: Path): List<String>
fun resolveManifestDependencyAliasList(manifestFile: File): List<String>
```

**Error handling:** All manifest functions throw `ActrException.Config` when the
manifest file cannot be parsed, a dependency alias is not found, or a dependency
lacks an `actr_type` field.

**Underlying FFI:** These functions wrap `io.actrium.actr.resolveManifestDependency`,
`resolveManifestDependencyAliasList`, and `resolveManifestPackageActrType` from the
UniFFI-generated layer, which call into the Rust `actr_config` manifest parser.

---

### NetworkEventHandle

Type alias: `typealias NetworkEventHandle = NetworkEventHandleWrapper`

Methods for notifying the runtime about platform events:

| Method | Description |
|--------|-------------|
| `suspend fun handleNetworkPathChanged(snapshot: NetworkSnapshot): NetworkEventResult` | Notify of network connectivity change |
| `suspend fun handleAppLifecycleChanged(state: AppLifecycleState): NetworkEventResult` | Notify of foreground/background transition |
| `suspend fun cleanupConnections(reason: CleanupReason): NetworkEventResult` | Request connection cleanup |
| `suspend fun forceReconnect(reason: ReconnectReason): NetworkEventResult` | Request forced reconnection |

**Result-wrapped extensions:**

```kotlin
suspend fun NetworkEventHandle.handleNetworkPathChangedCatching(snapshot): Result<NetworkEventResult>
suspend fun NetworkEventHandle.handleAppLifecycleChangedCatching(state): Result<NetworkEventResult>
suspend fun NetworkEventHandle.cleanupConnectionsCatching(reason): Result<NetworkEventResult>
suspend fun NetworkEventHandle.forceReconnectCatching(reason): Result<NetworkEventResult>
```

---

### NetworkMonitor (Android)

Android-specific network and lifecycle monitoring. Automatically forwards `ConnectivityManager` changes and app lifecycle transitions to the runtime.

```kotlin
// One-shot setup (node already created)
fun ActrNode.createNetworkMonitor(
    context: Context,
    scope: CoroutineScope,
    onNetworkStatusLog: ((String) -> Unit)? = null,
): NetworkMonitor

// Lazy setup (node created after monitor)
NetworkMonitor.create(
    context: Context,
    scope: CoroutineScope,
    getSystem: () -> ActrNode?,
    onNetworkStatusLog: ((String) -> Unit)? = null,
): NetworkMonitor
```

**Lifecycle methods:**

| Method | Description |
|--------|-------------|
| `fun startMonitoring()` | Start listening for network/lifecycle events |
| `fun stopMonitoring()` | Stop listening |
| `fun onAppBackground()` | Manually notify background transition |
| `fun onAppForeground()` | Manually notify foreground transition |
| `fun triggerNetworkCheck()` | Force a network status re-evaluation |

**Query methods:**

| Method | Description |
|--------|-------------|
| `fun isConnected(): Boolean` | Any network connectivity |
| `fun isWifi(): Boolean` | Connected via WiFi |
| `fun isCellular(): Boolean` | Connected via cellular |
| `fun isVpn(): Boolean` | Connected via VPN |
| `fun isEthernet(): Boolean` | Connected via Ethernet |
| `fun isNetworkExpensive(): Boolean` | Network is metered |
| `fun isNetworkConstrained(): Boolean` | Network is congested |
| `fun getCurrentNetworkStatus(): String` | Human-readable network status description |

---

### ActrException

Sealed class with 11 variants mirroring `actr_protocol::ActrError`.

| Variant | Description |
|---------|-------------|
| `Unavailable(msg)` | Peer or service unavailable |
| `TimedOut` | Request timed out |
| `NotFound(msg)` | Actor or route not found |
| `PermissionDenied(msg)` | Access denied |
| `InvalidArgument(msg)` | Invalid request parameters |
| `UnknownRoute(msg)` | Route not recognized |
| `DependencyNotFound(serviceName, detail)` | Required dependency missing |
| `DecodeFailure(msg)` | Payload deserialization failed |
| `NotImplemented(msg)` | Feature not implemented |
| `Internal(msg)` | Internal runtime error |
| `Config(msg)` | Configuration error |

**Extension properties:**

| Property | Description |
|----------|-------------|
| `userMessage: String` | Human-readable error description |
| `isTimeout: Boolean` | True for `TimedOut` |
| `isConnectionError: Boolean` | True for `Unavailable` |
| `isRecoverable: Boolean` | True if `ErrorKind.TRANSIENT` (worth retrying) |
| `kind: ErrorKind` | Fault domain: `TRANSIENT`, `CLIENT`, `INTERNAL`, or `CORRUPT` |
| `requiresDlq: Boolean` | True if `ErrorKind.CORRUPT` (route to dead-letter queue) |

---

### Retry Utilities

```kotlin
data class RetryConfig(
    val maxAttempts: Int = 3,
    val initialDelayMs: Long = 1000,
    val maxDelayMs: Long = 10000,
    val factor: Double = 2.0,
)

// Retry with exponential backoff
suspend fun <T> withRetry(
    maxAttempts: Int = 3,
    initialDelayMs: Long = 1000,
    maxDelayMs: Long = 10000,
    factor: Double = 2.0,
    shouldRetry: (Exception) -> Boolean = { it is ActrException && it.isRecoverable },
    block: suspend () -> T,
): T

// Retry with config object
suspend fun <T> withRetry(
    config: RetryConfig,
    shouldRetry: (Exception) -> Boolean = { it is ActrException && it.isRecoverable },
    block: suspend () -> T,
): T
```

---

## Low-Level API (Generated Bindings)

The low-level API is in `io.actrium.actr` and consists of UniFFI-generated bindings directly from the Rust codebase. In most cases, use the high-level DSL API instead.

### Key Generated Classes

| Class | Description |
|-------|-------------|
| `ActrNode` (in `io.actrium.actr`) | Raw node — use `ActrNode` from DSL layer instead |
| `ActrRefWrapper` | Raw actor reference — use `ActrRef` from DSL layer instead |
| `ContextBridge` | Workload context (same class used in both layers) |
| `DynamicWorkload` | Raw composite workload |
| `NetworkEventHandleWrapper` | Raw network event handle |
| `OpusEncoder` | Opus audio encoder |

### Key Generated Structs

| Struct | Fields |
|--------|--------|
| `ActrType` | `manufacturer: String`, `name: String`, `version: String` |
| `ActrId` | `realm: Realm`, `serialNumber: ULong`, `type: ActrType` |
| `Realm` | `realmId: UInt` |
| `DataStream` | `streamId: String`, `sequence: ULong`, `payload: ByteArray`, `metadata: List<MetadataEntry>`, `timestampMs: Long?` |
| `MetadataEntry` | `key: String`, `value: String` |
| `RpcEnvelopeBridge` | `routeKey: String`, `payload: ByteArray`, `requestId: String` |
| `NetworkSnapshot` | `sequence: ULong`, `availability: NetworkAvailability`, `transport: NetworkTransportFlags`, `isExpensive: Boolean`, `isConstrained: Boolean` |
| `NetworkTransportFlags` | `wifi: Boolean`, `cellular: Boolean`, `ethernet: Boolean`, `vpn: Boolean`, `other: Boolean` |
| `NetworkEventResult` | `event: NetworkEvent`, `success: Boolean`, `error: String?`, `durationMs: ULong` |
| `ErrorEventBridge` | `source: String`, `category: ErrorCategoryBridge`, `context: String`, `timestampMs: Long` |
| `BackpressureEventBridge` | `queueLen: ULong`, `threshold: ULong` |
| `CredentialEventBridge` | `newExpiryMs: Long` |
| `PeerEventBridge` | `peer: ActrId`, `relayed: Boolean?` |
| `MediaSample` | `trackId: String`, `data: ByteArray`, `timestampUs: ULong` |

### Key Generated Enums

| Enum | Variants |
|------|----------|
| `PayloadType` | `RPC_RELIABLE`, `RPC_SIGNAL`, `STREAM_RELIABLE`, `STREAM_LATENCY_FIRST`, `MEDIA_RTP` |
| `ErrorKind` | `TRANSIENT`, `CLIENT`, `INTERNAL`, `CORRUPT` |
| `ErrorCategoryBridge` | `HANDLER_PANIC`, `HANDLER_ERROR`, `SIGNALING_FAILURE`, `TRANSPORT_FAILURE`, `DATA_STREAM_DELIVERY_UNCERTAIN` |
| `NetworkAvailability` | `UNKNOWN`, `AVAILABLE`, `UNAVAILABLE` |
| `AppLifecycleState` | `Background`, `Foreground(backgroundDurationMs)` |
| `CleanupReason` | `APP_TERMINATING`, `USER_LOGOUT`, `STALE_CONNECTION_SUSPECTED`, `MANUAL_RESET` |
| `ReconnectReason` | `NETWORK_PATH_CHANGED`, `LONG_BACKGROUND`, `PROBE_FAILED`, `MANUAL_RECONNECT`, `STALE_CONNECTION_SUSPECTED` |
| `MediaType` | `AUDIO`, `VIDEO` |

### Observer Callback Interfaces

| Interface | Methods | Purpose |
|-----------|---------|---------|
| `SignalingObserverBridge` | `onConnecting(ctx?)`, `onConnected(ctx?)`, `onDisconnected(ctx)` | Signaling layer events |
| `WebSocketObserverBridge` | `onConnecting(ctx, event)`, `onConnected(ctx, event)`, `onDisconnected(ctx, event)` | WebSocket peer events |
| `WebRtcObserverBridge` | `onConnecting(ctx, event)`, `onConnected(ctx, event)`, `onDisconnected(ctx, event)` | WebRTC peer events |
| `CredentialObserverBridge` | `onRenewed(ctx, event)`, `onExpiring(ctx, event)` | Credential lifecycle |
| `MailboxObserverBridge` | `onBackpressure(ctx, event)` | Mailbox backpressure |
| `DataStreamCallback` | `onStream(chunk, sender)` | Incoming data stream chunk |
| `MediaTrackCallback` | `onSample(sample, sender)` | Incoming media sample |

### Free Functions

| Function | Description |
|----------|-------------|
| `actrErrorKind(e: ActrException): ErrorKind` | Classify fault domain |
| `actrErrorIsRetryable(e: ActrException): Boolean` | Check if transient/retryable |
| `actrErrorRequiresDlq(e: ActrException): Boolean` | Check if corrupt (dead-letter queue) |