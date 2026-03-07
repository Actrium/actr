# Service Management Architecture

This module manages the lifecycle of all services in the actrix node.

## Service Types

### ICE Services (UDP)

- **StunService** — standalone STUN server
- **TurnService** — TURN server (includes built-in STUN)

### HTTP Router Services (shared HTTP server)

- **SignalingService** — WebRTC signaling (`/signaling`)
- **AisService** — Actor Identity Service (`/ais`)
- **Control** — always-on control plane (`/admin` + Admin UI)

KS (Key Server) runs as a gRPC service mounted on the shared HTTP port.

## Core Abstractions

### HttpRouterService Trait

HTTP services produce an axum `Router` that is merged into the shared HTTP server:

```rust
#[async_trait]
pub trait HttpRouterService: Send + Sync + Debug {
    fn info(&self) -> &ServiceInfo;
    fn info_mut(&mut self) -> &mut ServiceInfo;
    async fn build_router(&mut self) -> Result<Router>;
    fn route_prefix(&self) -> &str;
}
```

### ServiceContainer Enum

Concrete service wrappers:

```rust
pub enum ServiceContainer {
    Signaling(SignalingService),
    Ais(AisService),
    Stun(StunService),
    Turn(TurnService),
}
```

### ServiceManager

Orchestrates all services:

- Builds a combined `Router` from all enabled HTTP services + control plane
- Serves via a single HTTP/HTTPS listener with hot-swap support (`watch::channel`)
- Starts ICE services on independent UDP sockets
- Manages config reload (SIGHUP), TLS certificate refresh, and ICE restart
- Wires `ServiceCounters` for per-service metrics collection

## Key Design Points

- **Hot-reload**: `build_router_from_config()` rebuilds the Router; new TCP connections use updated routes while existing ones continue unaffected.
- **Metrics**: Each service gets an `Arc<ServiceCounters>` for tracking active connections, requests, failures, and latency. A background sampler snapshots these into a 3-tier ring buffer.
- **Service enable bitmask**: Services are enabled/disabled via the `enable` config field. The control plane is always active.
