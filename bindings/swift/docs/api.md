# Actr API Reference

This document provides a comprehensive overview of the Actr API, organized into Low Level and High Level APIs.

## Overview

Actr provides two levels of API abstraction:

- **Low Level API** (`Actr` module): Direct FFI bindings to the Rust `actr` crate, providing fine-grained control
- **High Level API** (`Actr` module): Swift-friendly wrappers with type safety and concurrency guarantees

## Low Level API

The Low Level API is located in the `Actr` module and consists of UniFFI-generated bindings directly from the Rust codebase. These APIs provide direct access to the underlying Rust implementation.

### Core Wrapper Types

#### `ActrNode` (low-level, from `ActrBindings`)

Node-level wrapper for creating and starting package-backed ACTR nodes.

**Methods:**

- `static func newFromPackageFile(configPath: String, packagePath: String) async throws -> ActrNode`
  - Creates a new ACTR node from a TOML configuration file and a `.actr` package file
  - **Parameters:**
    - `configPath`: Path to the TOML configuration file
    - `packagePath`: Path to the `.actr` package file
  - **Returns:** An `ActrNode` instance (low-level `ActrBindings.ActrNode`)
  - **Throws:** `ActrError.Config` if the configuration is invalid

- `func createNetworkEventHandle() throws -> NetworkEventHandleWrapper`
  - Creates a network event handle for platform callbacks before startup
  - **Returns:** A `NetworkEventHandleWrapper` instance
  - **Throws:** `ActrError` if the runtime node is unavailable

- `func start() async throws -> ActrRefWrapper`
  - Starts the package-backed actor and returns a running reference
  - **Returns:** An `ActrRefWrapper` instance
  - **Throws:** `ActrError` if startup fails

#### `ActrRefWrapper`

Wrapper for a reference to a running actor. Provides methods for RPC calls, discovery, and lifecycle management.

**Methods:**

- `func actorId() -> ActrId`
  - Gets the actor's unique identifier
  - **Returns:** The actor's `ActrId`

- `func call(routeKey: String, payloadType: PayloadType, requestPayload: Data, timeoutMs: Int64) async throws -> Data`
  - Performs an RPC call to a remote actor via the RPC proxy mechanism
  - **Parameters:**
    - `routeKey`: RPC route key (e.g., "echo.EchoService/Echo")
    - `payloadType`: Payload transmission type (e.g., `.rpcReliable`, `.rpcSignal`)
    - `requestPayload`: Request payload bytes (protobuf encoded)
    - `timeoutMs`: Timeout in milliseconds
  - **Returns:** Response payload bytes (protobuf encoded)
  - **Throws:** `ActrError.Internal` if the call fails

- `func discover(targetType: ActrType, count: UInt32) async throws -> [ActrId]`
  - Discovers actors of the specified type
  - **Parameters:**
    - `targetType`: The type of actor to discover
    - `count`: Maximum number of actors to discover
  - **Returns:** Array of discovered actor IDs
  - **Throws:** `ActrError` if discovery fails

- `func tell(routeKey: String, payloadType: PayloadType, messagePayload: Data) async throws`
  - Sends a one-way message without expecting a response
  - **Parameters:**
    - `routeKey`: RPC route key (e.g., "echo.EchoService/Echo")
    - `payloadType`: Payload transmission type (e.g., `.rpcReliable`, `.rpcSignal`)
    - `messagePayload`: Message payload bytes (protobuf encoded)
  - **Throws:** `ActrError` if sending fails

- `func isShuttingDown() -> Bool`
  - Checks if the actor is currently shutting down
  - **Returns:** `true` if shutting down, `false` otherwise

- `func shutdown()`
  - Triggers actor shutdown (non-blocking)

- `func waitForShutdown() async`
  - Waits for the actor shutdown to complete

#### `ContextBridge`

Context provided to workloads during lifecycle callbacks. Provides access to RPC and discovery functionality.

**Methods:**

- `func callRaw(target: ActrId, routeKey: String, payloadType: PayloadType, payload: Data, timeoutMs: Int64) async throws -> Data`
  - Calls a remote actor via RPC (simplified for FFI)
  - **Parameters:**
    - `target`: Target actor ID
    - `routeKey`: RPC route key
    - `payloadType`: Payload transmission type (e.g., `.rpcReliable`, `.rpcSignal`)
    - `payload`: Request payload bytes
    - `timeoutMs`: Timeout in milliseconds
  - **Returns:** Response payload bytes
  - **Throws:** `ActrError.Internal` if the call fails

- `func discover(targetType: ActrType) async throws -> ActrId`
  - Discovers a single actor of the specified type
  - **Parameters:**
    - `targetType`: The type of actor to discover
  - **Returns:** The discovered actor ID
  - **Throws:** `ActrError` if discovery fails

- `func registerStream(streamId: String, callback: DataStreamCallback) async throws`
  - Registers a `DataStreamCallback` for the specified stream ID
  - **Parameters:**
    - `streamId`: Stream identifier to associate with the callback
    - `callback`: Callback interface invoked on incoming stream chunks
  - **Throws:** `ActrError` if registration fails

- `func sendDataStream(target: ActrId, chunk: DataStream) async throws`
  - Sends a data stream to a remote actor
  - **Parameters:**
    - `target`: Target actor ID
    - `chunk`: Data stream to send
  - **Throws:** `ActrError` if sending fails

- `func tellRaw(target: ActrId, routeKey: String, payloadType: PayloadType, payload: Data) async throws`
  - Sends a message to a remote actor without expecting a response (fire-and-forget)
  - **Parameters:**
    - `target`: Target actor ID
    - `routeKey`: Route key for the message
    - `payloadType`: Payload transmission type (e.g., `.rpcReliable`, `.rpcSignal`)
    - `payload`: Message payload bytes
  - **Throws:** `ActrError` if sending fails

- `func unregisterStream(streamId: String) async throws`
  - Unregisters a `DataStreamCallback` for the specified stream ID
  - **Parameters:**
    - `streamId`: Stream identifier to unregister
  - **Throws:** `ActrError` if unregistration fails

#### `WorkloadBridge`

Low-level callback interface retained in the generated bindings. The package-first runtime flow does not use it directly.

### Data Types

#### `ActrId`

Actor identifier structure.

```swift
public struct ActrId: Equatable, Hashable {
    public var realm: Realm
    public var serialNumber: UInt64
    public var type: ActrType
}
```

#### `ActrType`

Actor type identifier (manufacturer + name).

```swift
public struct ActrType: Equatable, Hashable {
    public var manufacturer: String
    public var name: String
}
```

#### `PayloadType`

Payload routing hints for RPC and streaming messages.

```swift
public enum PayloadType: Int32, Sendable {
    case rpcReliable = 0
    case rpcSignal = 1
    case streamReliable = 2
    case streamLatencyFirst = 3
    case mediaRtp = 4
}
```

#### `DataStream`

Data stream structure for fast-path streaming.

```swift
public struct DataStream: Equatable, Hashable {
    // Contains stream_id, sequence, payload, metadata, timestamp
}
```

#### `MetadataEntry`

Metadata entry for data streams.

```swift
public struct MetadataEntry: Equatable, Hashable {
    // Key-value metadata pair
}
```

#### `RpcEnvelopeBridge`

Envelope passed to workloads when dispatching inbound RPC messages.

```swift
public struct RpcEnvelopeBridge: Sendable {
    public let routeKey: String
    public let payload: Data
    public let requestId: String
}
```

#### `Realm`

Realm identifier.

```swift
public struct Realm: Equatable, Hashable {
    // Realm identifier
}
```

#### `ActrError`

Error type for ACTR operations. Mirrors `actr_protocol::ActrError` 1:1, with
one binding-local variant (`Config`) for pre-protocol configuration failures.

```swift
public enum ActrError: Swift.Error, Equatable, Hashable {
    // Transient — retry with backoff
    case Unavailable(msg: String)
    case TimedOut

    // Client — fail fast
    case NotFound(msg: String)
    case PermissionDenied(msg: String)
    case InvalidArgument(msg: String)
    case UnknownRoute(msg: String)
    case DependencyNotFound(serviceName: String, message: String)

    // Corrupt — route to Dead Letter Queue
    case DecodeFailure(msg: String)

    // Internal — framework bug / panic
    case NotImplemented(msg: String)
    case Internal(msg: String)

    // Binding-local (pre-protocol)
    case Config(msg: String)
}
```

Classification helpers (free functions, not methods — UniFFI limitation):

```swift
let kind: ErrorKind = actrErrorKind(err)             // .transient / .client / .internal / .corrupt
let retry: Bool = actrErrorIsRetryable(err)          // true iff kind == .transient
let dlq: Bool = actrErrorRequiresDlq(err)            // true iff kind == .corrupt
```

## High Level API

The High Level API is located in the `Actr` module and provides Swift-friendly wrappers with improved type safety, concurrency guarantees, and Protobuf integration.

### Core Types

#### `ActrNode`

High-level entry point for creating and starting a package-backed ACTR node. This is a `Sendable` class, making it safe to pass across concurrency boundaries.

**Methods:**

- `static func from(packageConfig path: String, packagePath: String) async throws -> ActrNode`
  - Creates a node from a TOML config file path and `.actr` package path
  - **Parameters:**
    - `path`: Path to the TOML configuration file
    - `packagePath`: Path to the `.actr` package file
  - **Returns:** An `ActrNode` instance
  - **Throws:** `ActrError.Config` if the configuration is invalid

- `static func from(packageConfig configURL: URL, packageURL: URL) async throws -> ActrNode`
  - Creates a node from TOML config and `.actr` package URLs
  - **Parameters:**
    - `configURL`: File URL to the TOML configuration file
    - `packageURL`: File URL to the `.actr` package file
  - **Returns:** An `ActrNode` instance
  - **Throws:** `ActrError.Config` if the URL is not a file URL or configuration is invalid

- `func start() async throws -> ActrRef`
  - Starts the package-backed actor and returns a high-level actor reference
  - **Returns:** An `ActrRef` actor instance
  - **Throws:** `ActrError` if startup fails

#### `ActrRef`

A concurrency-safe reference to a running ACTR actor. This is an `actor` type, providing automatic concurrency safety through Swift's actor isolation.

**Properties:**

- `var id: ActrId { get }`
  - Returns the actor ID of this running actor (read-only)

**Methods:**

- `func call<Req: RpcRequest>(_ message: Req, payloadType: PayloadType = .rpcReliable, timeoutMs: Int64 = 30_000) async throws -> Req.Response`
  - Performs a type-safe Protobuf-based RPC call
  - **Type Parameters:**
    - `Req`: Request message type conforming to `RpcRequest`
  - **Parameters:**
    - `message`: Request message instance
    - `payloadType`: Payload transmission type (defaults to `.rpcReliable`)
    - `timeoutMs`: Timeout in milliseconds
  - **Returns:** Response message instance (`Req.Response`)
  - **Throws:** 
    - `ActrError.Internal` if `Req.routeKey` is empty
    - `ActrError.Internal` if the call fails
  - **Note:** This method automatically handles Protobuf serialization/deserialization

- `func discover(type: ActrType, limit: Int = 1) async throws -> [ActrId]`
  - Discovers actors of the given type
  - **Parameters:**
    - `type`: The type of actor to discover
    - `limit`: Maximum number of actors to discover (default: 1)
  - **Returns:** Array of discovered actor IDs (empty if limit is 0)
  - **Throws:**
    - `ActrError.Internal` if limit is invalid
  - **Note:** Uses Swift `Int` instead of `UInt32` for better ergonomics

- `func stop() async`
  - Shuts down the actor and waits for it to terminate
  - This is a non-throwing method that ensures clean shutdown

### Exposed Types

The following types are available in the `Actr` module:

- `ActrError`
- `ActrId`
- `ActrType`
- `PayloadType`
- `Realm`
- `DataStream`
- `DataStreamCallback`
- `MetadataEntry`
- `Context` (alias of `ContextBridge`)
- `RpcEnvelope` (alias of `RpcEnvelopeBridge`)
- `Workload` (alias of `WorkloadBridge`, low-level only)
- `RpcRequest` (built into `Actr`)

## API Comparison

| Feature | Low Level API | High Level API |
|---------|---------------|----------------|
| **Module** | `Actr` (FFI types) | `Actr` (high-level) |
| **RPC Calls** | `call()` with raw `Data` bytes | `call()` with type-safe Protobuf `Message` |
| **Concurrency Safety** | Manual management required | `ActrRef` is an `actor` type with automatic isolation |
| **Parameter Types** | Uses low-level types (e.g., `UInt32`) | Uses Swift-friendly types (e.g., `Int`) |
| **Error Handling** | Direct `ActrError` throwing | Same, but with better integration |
| **Protobuf Integration** | Manual serialization/deserialization | Automatic via generic constraints |
| **Use Case** | Direct FFI control, advanced scenarios | Daily development, recommended for most use cases |

## Usage Recommendations

### When to Use High Level API

- **Recommended for most use cases**: The High Level API provides better type safety, automatic Protobuf handling, and concurrency guarantees
- **Protobuf-based RPC**: When you're using Protobuf messages, the type-safe `call()` method eliminates serialization boilerplate
- **Swift concurrency**: The `ActrRef` actor type integrates seamlessly with Swift's structured concurrency

### When to Use Low Level API

- **Direct control**: When you need fine-grained control over the FFI layer
- **Custom serialization**: When you're not using Protobuf or need custom serialization logic
- **Advanced scenarios**: When you need access to features not yet exposed in the High Level API

## Example Usage

### High Level API Example

```swift
import Actr
import SwiftProtobuf

// Create node from config + package
let node = try await ActrNode.from(
    packageConfig: "/path/to/config.toml",
    packagePath: "/path/to/app.actr"
)

// Start the package-backed actor
let actrRef = try await node.start()

// Type-safe RPC call with Protobuf
let request = EchoRequest.with { $0.message = "Hello" }
let response: EchoResponse = try await actrRef.call(request)
print(response.reply)

// Discover actors
let actors = try await actrRef.discover(type: echoType, limit: 5)

// Clean shutdown
await actrRef.stop()
```

### Low Level API Example

```swift
import Actr
import ActrBindings

// Create node (low-level wrapper)
let node = try await ActrBindings.ActrNode.newFromPackageFile(
    configPath: "/path/to/config.toml",
    packagePath: "/path/to/app.actr"
)

// Start actor
let refWrapper = try await node.start()

// Raw RPC call with Data
let requestData = try request.serializedData()
let responseData = try await refWrapper.call(
    routeKey: "echo.EchoService/Echo",
    payloadType: .rpcReliable,
    requestPayload: requestData,
    timeoutMs: 30_000
)
let response = try EchoResponse(serializedBytes: responseData)

// Discover actors
let actors = try await refWrapper.discover(targetType: echoType, count: 5)

// Shutdown
refWrapper.shutdown()
await refWrapper.waitForShutdown()
```

## Additional Resources

- [Actr README](../README.md) - Package overview and build instructions
- [Echo App Example](../../echo-app/README.md) - Example iOS application using Actr
