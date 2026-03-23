# actr-runtime Internal Architecture

**Document Purpose**: Provide an internal architectural view of runtime-related crates for contributors and advanced users.

**Last Updated**: 2026-03-11
**Corresponding Version**: actr v0.9.x

---

## 1. Module Overview

The Actor-RTC runtime consists of two crates working together:

- **actr-runtime**: Pure business dispatch layer (ACL + dispatch + lifecycle hooks), no IO dependencies, compilable on native and `wasm32-unknown-unknown` targets.
- **actr-hyper**: Infrastructure layer + platform layer, carrying transport, Wire, signaling, WASM/Dynclib engines, and Actor sandbox management.

```
actr-hyper   ← Infrastructure Layer (transport, wire, signaling, WASM engine, dynclib engine …)
actr-runtime ← Business Dispatch Layer (ACL + dispatch + lifecycle hooks)
actr-framework ← SDK Interface Layer (trait definitions: Workload, Context, MessageDispatcher)
actr-protocol  ← Data Definition Layer (protobuf types)
```

### actr-runtime Directory Structure

```
actr-runtime/
├── acl.rs              # ACL permission checks (pure functions, no IO)
├── dispatch.rs         # ActrDispatch: ACL → routing → handler execution
└── lib.rs              # Re-exports Workload, Context, MessageDispatcher
```

### actr-hyper Directory Structure (Runtime Infrastructure)

```
actr-hyper/
├── lifecycle/          # Actor Lifecycle Management
│   ├── actr_system.rs  # ActrSystem (generic-agnostic infrastructure)
│   └── actr_node.rs    # ActrNode<W> (complete node)
├── inbound/            # Inbound Message Processing
│   ├── data_stream_registry.rs     # DataStream fast path registry
│   └── media_frame_registry.rs     # MediaFrame fast path registry
├── outbound/           # Outbound Message Processing
│   ├── host_gate.rs    # In-process outbound gate (Shell ↔ Workload)
│   └── peer_gate.rs    # Cross-process outbound gate (Actor ↔ Actor)
├── transport/          # Transport Layer Abstraction
│   ├── lane.rs              # DataLane unified abstraction
│   ├── route_table.rs       # PayloadType routing table
│   ├── manager.rs           # TransportManager trait
│   ├── inproc_manager.rs    # HostTransport in-process transport management
│   ├── dest_transport.rs    # Destination transport abstraction
│   ├── wire_pool.rs         # Wire connection pool
│   └── wire_handle.rs       # Wire handle
├── wire/               # Underlying Transport Protocols
│   ├── webrtc/              # WebRTC implementation
│   │   ├── coordinator.rs   # WebRTC coordinator
│   │   ├── gate.rs          # WebRTC gate (inbound)
│   │   ├── connection.rs    # WebRTC connection
│   │   ├── negotiator.rs    # SDP negotiator
│   │   └── signaling.rs     # Signaling client
│   └── websocket/           # WebSocket implementation
│       ├── connection.rs    # WebSocket connection
│       ├── gate.rs          # WebSocket inbound gate
│       └── server.rs        # WebSocket server
├── workload.rs         # Workload runtime abstraction (WASM/Dynclib unified dispatch interface)
├── wasm/               # WASM engine (feature: wasm-engine)
├── dynclib/            # Dynclib engine (feature: dynclib-engine)
├── context.rs          # Context implementation
├── context_factory.rs  # Context factory
├── actr_ref.rs         # ActrRef (Actor reference)
└── runtime_error.rs    # Error type definitions
```

---

## 2. Runtime Workload Backends

Hyper currently supports two runtime workload backends.

### 2.1 WASM Integration

.wasm modules are loaded by WasmHost, implementing asynchronous I/O via asyncify suspend/resume.

- **Dispatch Path**: `ActrNode::handle_incoming` → `Workload::handle` → `WasmWorkload` (wasmtime)
- **Scheduling**: Dynamic dispatch via `Workload` enum
- **Feature Gate**: `wasm-engine`
- **I/O Model**: Guest calls `actr_host_invoke` host import → asyncify suspend → Host completes I/O → Resume execution
- **Use Case**: Third-party Actor sandbox isolation, cross-platform distribution

### 2.2 Dynclib Integration

.so / .dylib / .dll native shared libraries loaded by DynclibHost, invoked via C ABI + VTable.

- **Dispatch Path**: `ActrNode::handle_incoming` → `Workload::handle` → `DynClibWorkload` (dlopen)
- **Scheduling**: Dynamic dispatch via `Workload` enum
- **Feature Gate**: `dynclib-engine`
- **Performance**: Close to Source mode (native code), but with FFI boundary overhead
- **Use Case**: Actors requiring native performance while maintaining independent deployment

### Dispatch Path Summary

```
handle_incoming(envelope)
    │
    ├── self.workload == Some(workload)
    │   └── workload.handle(bytes, ctx, host_abi)           // WASM / Dynclib
    │
    └── self.workload == None
        └── shell-only node (no guest inbound dispatch)
```

---

## 3. Two Transport Strategies

Transport strategies are orthogonal to integration modes — any integration mode can use either transport strategy.

### 3.1 HostGate + HostTransport (Shell ↔ Workload)

- **Responsibility**: Bidirectional communication between Shell and Workload within the same process
- **Implementation**: `tokio::sync::mpsc` channel, zero serialization
- **Latency**: ~10μs
- **Bidirectional Design**: Two independent HostTransport instances
  - Shell → Workload (REQUEST)
  - Workload → Shell (RESPONSE)

### 3.2 PeerGate + PeerTransport (Actor ↔ Actor)

- **Responsibility**: Cross-process / Cross-network inter-Actor communication
- **Implementation**: WebRTC DataChannel / WebSocket
- **Serialization**: Protobuf
- **Latency**: 1-50ms (network dependent)
- **pending_requests**: Manages RPC request-response matching

---

## 4. Workload runtime abstraction

`Workload` is the unified runtime entry for WASM and Dynclib.

```rust
pub enum Workload {
    Wasm(WasmWorkload),
    DynClib(DynClibWorkload),
}

impl Workload {
    fn handle<'a>(
        &'a mut self,
        request_bytes: &[u8],
        ctx: InvocationContext,
        host_abi: &'a HostAbiFn,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, ...>> + Send + 'a>>;
}
```

### Shared Types

- **InvocationContext**: Context for each request (self_id, caller_id, request_id)
- **HostOperation**: Outbound call enumeration initiated by Guest (Call / Tell / Discover / CallRaw)
- **HostOperationResult**: Outbound I/O operation result (Bytes / Done / Error)
- **HostAbiFn**: Closure for executing outbound calls on the host side

---

## 5. Layered Architecture

The runtime adopts a 4-layer architecture design:

```
┌─────────────────────────────────────────────┐
│  Layer 3: Application (Workload)            │  User business logic
│    Inbound: DataStreamRegistry              │  Fast path callbacks
│             MediaFrameRegistry              │
├─────────────────────────────────────────────┤
│  Layer 2: Outbound Gate                     │  Outbound gate abstraction
│    Gate::Host(Arc<HostGate>)                │  Shell ↔ Workload
│    Gate::Peer(Arc<PeerGate>)                │  Actor ↔ Actor
├─────────────────────────────────────────────┤
│  Layer 1: Transport (DataLane)              │  Transport channel abstraction
│    HostTransport                            │  In-process mpsc
│    PeerTransport                            │  WebRTC/WebSocket
│    WirePool / WireHandle                    │
├─────────────────────────────────────────────┤
│  Layer 0: Wire (Protocol)                   │  Physical transport protocol
│    WebRTC (DataChannel + RTP)               │
│    WebSocket                                │
│    tokio::sync::mpsc                        │
└─────────────────────────────────────────────┘
```

### Gate enum

```rust
pub enum Gate {
    Host(Arc<HostGate>),   // In-process transport (zero serialization)
    Peer(Arc<PeerGate>),   // Cross-process transport (Protobuf serialization)
}
```

Design Advantage: enum dispatch provides static dispatch with zero virtual function call overhead, CPU branch prediction hit rate >95%.

### Key Design Principles

1. **Layer Separation**: Each layer depends only on the layer below it, no cross-layer calls.
2. **Unified Abstraction**: Unified Host and Peer paths via DataLane.
3. **Separation of Semantics and Capability**: PayloadType determines top-level semantics; specific execution strategy is decided by resolver based on semantics and backend capabilities.
4. **Zero-Cost Abstraction**: Zero serialization for Host path, zero copy (`Bytes` shallow copy) for Peer path.

---

## 6. Core Component Responsibilities

### 6.1 Business Dispatch Layer (actr-runtime)

**ActrDispatch<W>**：
- Responsibility: Pure business dispatcher — ACL check → routing → handler execution → panic capture
- No IO dependency, compilable on native and wasm32 targets
- Key methods: `dispatch()`, `on_start()`, `on_stop()`

**check_acl_permission**：
- Responsibility: Pure function ACL permission judgment
- Evaluation Rules: Local call allowed → No ACL allowed → Empty rule rejected → Deny-first → Allow hit allowed → Default denied

### 6.2 Lifecycle Management (actr-hyper/lifecycle/)

**ActrSystem**：
- Responsibility: Provides workload-agnostic infrastructure (Mailbox, SignalingClient, ContextFactory)
- Lifecycle: Converts to ActrNode from creation to `attach_workload()` or `attach_shell()`
- Key methods: `from_config()`, `new()`, `attach_workload()`, `attach_shell()`

**ActrNode**：
- Responsibility: Runtime node, optionally holding one guest workload plus runtime components
- Dispatch Path: Uses `Workload::handle(...)` when a workload is attached
- Key methods: `start()`, `handle_incoming()`, `shutdown()`
- Key fields: `workload: Option<Mutex<Workload>>`

### 6.3 Inbound Processing (actr-hyper/inbound/)

**WebRtcGate**：
- Responsibility: Consumes inbound data aggregated by `WebRtcCoordinator`, dispatching directly based on PayloadType
- Routing Rules:
  - RpcReliable/RpcSignal → Check pending_requests first; if hit, treat as Response and wake continuation, otherwise enter Mailbox by priority
  - StreamReliable/StreamLatencyFirst → DataStreamRegistry (fast path callback)
  - MediaRtp → Drop directly and hint to use WebRTC Track (MediaFrameRegistry registered by PeerConnection)

**Inproc Receive Loop**：
- Responsibility: Two tokio loops inside `ActrNode` (Shell→Workload, Workload→Shell) receive directly from `HostTransport`'s `DataLane::Mpsc`
- Shell→Workload: Extract `RpcEnvelope` then call `handle_incoming()`
- Workload→Shell: Call `complete_response()` based on `request_id` to wake requester

**DataStreamRegistry**：
- Responsibility: Manage DataStream callback registry (stream_id → callback)
- Concurrency Safety: Use DashMap to support multi-threaded concurrent access
- Callback Signature: `FnMut(DataStream, ActrId) -> BoxFuture<ActorResult<()>>`

**MediaFrameRegistry**：
- Responsibility: Manage MediaTrack callback registry (track_id → callback)
- Concurrency Safety: Use DashMap
- Callback Signature: `FnMut(MediaSample, ActrId) -> BoxFuture<ActorResult<()>>`

#### Semantic Decision Model (Current Constraints & Evolution Direction)

> Note: This section describes the recommended unified decision model inside runtime, explaining current implementation and guiding future `wasm` / `StateSync` extensions; where `StateSync` and some backend-specific policies are still design directions, not yet fully implemented.

- `PayloadType` expresses top-level data semantics, not solely determining local execution mode:
  - `RpcReliable`
  - `RpcSignal`
  - `StreamReliable`
  - `StreamLatencyFirst`
  - `MediaRtp`
  - `StateSync` (planned)
- `MessageRole` expresses interaction role:
  - `Request`
  - `Response`
  - `Notify`
  - `Data`
  - `Snapshot`
  - `Delta`
- Backend capabilities split into two orthogonal dimensions, rather than mixed into a single `BackendProfile`:
  - `RuntimeKind`: `native` | `wasm`
  - `TransportKind`: `inproc` | `webrtc` | `websocket`
- `ExecutionPolicy` is the output of the resolver, not an input dimension:
  - `Mailbox`
  - `PendingContinuation`
  - `OrderedStreamQueue`
  - `CoalescingQueue`
  - `MediaPipeline`
  - `LatestValueStore`

Recommended Solution Form:

```rust
ExecutionPlan = resolve(payload_type, message_role, runtime_kind, transport_kind, hints)
```

- `hints` used only for tuning parameters, not overriding core semantics. Typical fields:
  - `priority`
  - `queue_depth`
  - `batch_size`
  - `ttl/deadline`
  - `drop_policy`
  - `persistence`
- Resolver needs to check combination validity first; not all combinations are valid, e.g.:
  - `MediaRtp + Response`: Usually invalid
  - `RpcReliable + Data`: Usually invalid
  - `StateSync + Request`: Usually shouldn't be default combination

Recommended Default Mapping:

| Semantic Combination | Default ExecutionPolicy | Remarks |
| --- | --- | --- |
| `RpcReliable + Request/Notify` | `Mailbox` | Normal priority, enters actor state path |
| `RpcSignal + Request/Notify` | `Mailbox` | High priority control message |
| `RpcReliable/RpcSignal + Response` | `PendingContinuation` | Defaults to serving `call().await`; if "handle later as actor event" needed, model as explicit split-phase API, not rewriting normal response semantics |
| `StreamReliable + Data` | `OrderedStreamQueue` | Current implementation approximates `DataStreamRegistry` fast path, can add bounded queue / batching later |
| `StreamLatencyFirst + Data` | `CoalescingQueue` | Current implementation shares registry with `StreamReliable`, target semantics should be latest-first / coalescing |
| `MediaRtp + Data` | `MediaPipeline` | Should follow WebRTC Track fast path; `websocket` usually not a valid carrier path |
| `StateSync + Snapshot/Delta` | `LatestValueStore` | planned; Old values can be overwritten by new ones, shouldn't force reuse of RPC/mailbox semantics |

Design Constraints:

- `Response -> PendingContinuation` is default semantics for normal `call().await`, not absolutely prohibiting split-phase.
- If business needs "process response later / queue / persist", use explicit split-phase API (e.g., response to self-notify / workflow event), instead of changing all normal RPC responses to Mailbox events.
- `wasm` vs `native` differences should reflect in resolver-produced `ExecutionPlan`, not in developer API or `PayloadType` bifurcation. Especially `wasm` backend should prioritize reducing host/guest crossing, favoring batch / coalescing, rather than replicating native fine-grained scheduling.

### 6.4 Outbound Processing (actr-hyper/outbound/)

**Gate** enum:
- `Host(Arc<HostGate>)`: In-process outbound
- `Peer(Arc<PeerGate>)`: Cross-process outbound
- Design Advantage: Static dispatch, zero virtual call overhead

**HostGate**:
- Responsibility: Send in-process messages via HostTransport
- Features: Zero serialization, directly passes RpcEnvelope object
- Latency: ~10μs

**PeerGate**:
- Responsibility: Send cross-process messages via PeerTransport
- Features: Protobuf serialization, transmission via WebRTC/WebSocket
- Latency: 1-50ms (network dependent)
- pending_requests: Manages RPC request-response matching

### 6.5 Transport Layer (actr-hyper/transport/)

**DataLane** enum:
- `Mpsc { payload_type, tx, rx }`: In-process tokio mpsc channel
- `WebRtcDataChannel { data_channel, rx }`: WebRTC DataChannel
- `WebSocket { sink, payload_type, rx }`: WebSocket connection

**PayloadTypeExt** trait:
- Core Method: `data_lane_types() -> &'static [DataLaneType]`
- Function: Provides static routing table from PayloadType to DataLaneType
- Advantage: Determined at compile time, zero runtime overhead
- Note: `PayloadTypeExt` only solves "which lane to take", not independently deciding local `Mailbox / PendingContinuation / Registry / MediaPipeline` execution strategy; the latter is decided by semantic resolver above

**TransportManager** trait:
- Responsibility: Manage transport channel lifecycle (creation, caching, reuse)
- Implementation:
  - `HostTransport`: Manage in-process mpsc channel
  - `PeerTransport`: Manage WebRTC/WebSocket connections

### 6.6 Wire Layer (actr-hyper/wire/)

**WebRtcCoordinator**:
- Responsibility: Manage lifecycle of all WebRTC peer connections
- Key Functions:
  - Start multi-PayloadType receive loops (RpcReliable, RpcSignal, StreamReliable, StreamLatencyFirst)
  - Aggregate messages from all peers to unified message_rx
  - Provide `send_message()` and `receive_message()` interfaces

**WebRtcGate**:
- Responsibility: WebRTC inbound message routing (Coordinator → Mailbox/Registry)
- Routing Logic:
  - Dispatch messages based on PayloadType
  - RPC messages check pending_requests first: if hit complete continuation, otherwise enqueue(Mailbox) by priority
  - DataStream messages dispatched directly to DataStreamRegistry

**WebRtcConnection**:
- Responsibility: Encapsulate single RTCPeerConnection, manage DataChannel and MediaTrack
- Key Methods: `create_data_channel()`, `get_lane()`, `add_track()`

---

## 7. Key Data Flows

### 7.1 RPC Request-Response Flow

**Sender (PeerGate)**:
```rust
1. send_request(target, envelope)
2. Generate request_id, register oneshot::Sender to pending_requests
3. Serialize RpcEnvelope → Bytes
4. TransportManager → DataLane → WebRTC
```

**Receiver (WebRtcGate)**:
```rust
1. Coordinator.receive_message() → (from, data, RpcReliable)
2. Deserialize Bytes → RpcEnvelope
3. Check if request_id is in pending_requests
4. If Response: Wake oneshot::Sender
5. If Request: enqueue(Mailbox)
```

### 7.2 DataStream Fast Path Flow

**Sender**:
```rust
1. ctx.send_data_stream(target, stream_id, chunk)
2. Gate::send_data_stream(target, StreamReliable, data)
3. TransportManager → DataLane(StreamReliable) → WebRTC
```

**Receiver**:
```rust
1. Coordinator.receive_message() → (from, data, StreamReliable)
2. WebRtcGate identifies PayloadType::StreamReliable
3. Deserialize Bytes → DataStream
4. DataStreamRegistry.dispatch(chunk, sender_id)
5. Invoke registered callback function
```

### 7.3 WASM Actor Dispatch Flow

```rust
1. handle_incoming(envelope)
2. Detect self.workload == Some(workload)
3. Serialize envelope → bytes
4. workload.handle(bytes, InvocationContext, host_abi)
5. WASM guest execution → encounter actr_host_invoke → asyncify suspend
6. host_abi(HostOperation::Call { ... }) → HostOperationResult::Bytes(response)
7. asyncify resume → guest continues execution → return result bytes
```

---

## 8. Performance Optimization Design

### 8.1 Zero Copy Design

- **Host Path**: Pass `RpcEnvelope` object directly, no serialization
- **Peer Path**: Use `Bytes` type (Arc<Vec<u8>>), shallow copy
- **MediaTrack**: WebRTC native RTP channel, bypassing Protobuf serialization

### 8.2 Compile-Time Routing

- **PayloadTypeExt**: Routing table determined at compile time, no runtime lookup
- **Source Mode**: `ActrDispatch<W>` monomorphized via generics, fully inlined
- **Gate enum**: Static dispatch, superior to trait object

### 8.3 Fine-Grained Concurrency

- **DashMap**: Used for Registry, supporting high concurrency read/write
- **Independent Receive Loops**: Independent tokio task for each PayloadType
- **Lock-Free Design**: Use mpsc/oneshot where possible to avoid lock contention

---

## 9. Error Handling Strategy

### 9.1 Error Type Hierarchy

```rust
RuntimeError
├── TransportError      # Transport layer error (disconnect, timeout)
├── ProtocolError       # Protocol error (deserialization failure)
└── Other(anyhow::Error)  # Other errors
```

### 9.2 Error Propagation

- **Transport Error**: Automatic retry (with exponential backoff)
- **Protocol Error**: Log and drop message
- **Application Error**: Return to caller via RpcEnvelope.error

---

## 10. Testing Strategy

### 10.1 Unit Testing

- `actr-runtime/acl.rs`: ACL permission check tests
- `actr-runtime/dispatch.rs`: Dispatcher panic capture tests
- `transport/lane.rs`: DataLane creation and send/receive tests
- `inbound/data_stream_registry.rs`: Callback registration and trigger tests
- `outbound/host_gate.rs`: In-process message sending tests

### 10.2 Integration Testing

- `actr-hyper/tests/wasm_actor_e2e.rs`: WASM actor end-to-end tests
- `actr-hyper/tests/asyncify_poc.rs`: asyncify suspend/resume verification
- `actr-hyper/tests/wasm_host.rs`: WasmHost/WasmWorkload tests

---

## 11. Dependency Graph

```
actr-hyper
├── actr-runtime       (Business Dispatch Layer)
├── actr-framework     (Trait Definitions)
├── actr-protocol      (Protocol Definitions)
├── actr-runtime-mailbox (Persistent Mailbox)
├── actr-config        (Config Parsing)
├── tokio              (Async Runtime)
├── webrtc             (WebRTC Implementation)
├── tokio-tungstenite  (WebSocket Implementation)
├── wasmtime           (WASM Engine, optional feature: wasm-engine)
├── dashmap            (Concurrent HashMap)
└── anyhow             (Error Handling)

actr-runtime
├── actr-framework     (Trait Definitions)
├── actr-protocol      (Protocol Definitions)
├── bytes              (Zero-copy buffer)
├── futures-util       (catch_unwind)
└── tracing            (Logging)
```

---

## 12. Contribution Guidelines

### 12.1 Code Organization Principles

1. **Single Responsibility**: Each module responsible for one clear function
2. **Dependency Inversion**: High-level modules depend on abstractions (traits), not concrete implementations
3. **Open/Closed Principle**: Extend functionality via enum and trait, rather than modifying existing code

### 12.2 Naming Conventions

- **Manager**: Lifecycle management component (e.g., TransportManager)
- **Registry**: Callback registration management component (e.g., DataStreamRegistry)
- **Gate**: Message entry/exit abstraction (e.g., WebRtcGate, Gate)
- **Coordinator**: Component coordinating multiple related components (e.g., WebRtcCoordinator)
- **Bridge**: ABI bridge or transport bridge component

### 12.3 Pre-PR Checklist

- [ ] Unit tests passed
- [ ] Integration tests passed
- [ ] Updated relevant docs (README, ARCHITECTURE.md)
- [ ] Code complies with rustfmt and clippy standards
- [ ] No significant regression in performance-sensitive paths

---

## 13. References

- [User Docs: Runtime Design](../../actor-rtc.github.io/zh-hans/appendix-runtime-design.zh.md)
- [User Docs: Glossary](../../actor-rtc.github.io/zh-hans/appendix-glossary.zh.md)
- [User Docs: Lane Selection Strategy](../../actor-rtc.github.io/zh-hans/appendix-lane-selection-strategy.zh.md)
- [actr-protocol README](../protocol/README.md)
- [actr-framework README](../framework/README.md)

---

**Maintainers**: actr Core Team
**Issues**: https://github.com/actor-rtc/actr/issues
