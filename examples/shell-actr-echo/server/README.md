# Echo Real Server

Echo service server implementation using Actor-RTC framework.

## Architecture

This is a **server-only** application that:
- Implements `EchoServiceHandler` trait
- Registers with signaling server
- Waits for client connections
- Responds to Echo requests

## Directory Structure

```
echo-real-server/
├── proto/              # Protobuf service definitions
│   └── echo.proto     # EchoService definition
├── src/
│   ├── generated/     # Auto-generated code (DO NOT EDIT)
│   │   ├── echo.rs    # Protobuf message types
│   │   └── echo_service_actor.rs  # Actor framework code
│   ├── echo_service.rs  # Business logic implementation
│   └── main.rs        # Server entry point
└── Actr.toml          # Actor configuration

```

## Generated Code

The `src/generated/` directory contains:
- `echo.rs` - Protobuf message types (EchoRequest, EchoResponse)
- `echo_service_actor.rs` - Actor framework code:
  - `EchoServiceHandler` trait (to be implemented)
  - `EchoServiceWorkload` wrapper
  - `EchoServiceDispatcher` (auto-routing)
  - `Message` trait implementations

## Implementation

`src/echo_service.rs` implements the business logic:

```rust
#[async_trait]
impl EchoServiceHandler for EchoService {
    async fn echo<C: Context>(&self, req: EchoRequest, ctx: &C) -> ActorResult<EchoResponse> {
        // Business logic here
        Ok(EchoResponse {
            reply: format!("Echo: {}", req.message),
            timestamp: /* ... */
        })
    }
}
```

## Running

```bash
# Build
cargo build

# Run (requires signaling-server at ws://localhost:8081)
cargo run
```

## See Also

- [echo-real-client-app](../echo-real-client-app/) - Client application
- [shell-echo.sh](../../shell-echo.sh) - Full integration test script
