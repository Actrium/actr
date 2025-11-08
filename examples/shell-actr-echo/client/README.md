# Echo Real Client App

Interactive client application for testing Echo service using Actor-RTC framework.

## Architecture

This is a **client-only** application that:
- Runs as an Actor node (ClientWorkload)
- Connects to signaling server
- Calls remote Echo service
- Provides interactive CLI for user input

## Directory Structure

```
echo-real-client-app/
├── src/
│   ├── generated/       # Minimal generated code
│   │   ├── echo.rs     # Protobuf message types only
│   │   └── mod.rs
│   ├── message_impl.rs  # Manual Message trait implementation
│   ├── client_workload.rs  # Actor workload (message forwarding)
│   ├── app_side.rs      # User interaction layer (CLI)
│   └── main.rs          # Client entry point
└── Cargo.toml

```

## Design Principles

1. **No Server Code**: Client does NOT contain server implementation
2. **Minimal Generated Code**: Only protobuf messages (EchoRequest, EchoResponse)
3. **Manual Message Trait**: Implements `Message` trait manually for type-safe RPC
4. **Two-Layer Architecture**:
   - **App Side**: User interaction (stdin/stdout)
   - **Client Workload**: Actor node (message forwarding)

## Components

### 1. ClientWorkload (Actor Layer)

Runs as an Actor node, forwards messages from App to remote server:

```
App → call(local_id, request) → ClientWorkload
                                     ↓
                              ctx.call(server_id, request)
                                     ↓
                                Remote Server
```

### 2. AppSide (User Interaction)

Provides interactive CLI:
- Reads user input
- Calls local ClientWorkload
- Displays responses

### 3. Message Implementation

Manually implements `Message` trait for type-safe calls:

```rust
impl Message for EchoRequest {
    type Response = EchoResponse;
    fn route_key() -> &'static str {
        "echo.EchoService.Echo"
    }
}
```

## Running

```bash
# Build
cargo build

# Run (requires signaling-server and echo-real-server running)
cargo run
```

Example session:
```
===== Echo Client App =====
Type messages to send to server (type 'quit' to exit):
> Hello
[Received reply] Echo: Hello
> World
[Received reply] Echo: World
> quit
```

## Bidirectional Communication (ShellHandle)

### New Architecture (v0.2.0+)

`ActrNode::start()` now returns **two handles** for bidirectional communication:

```rust
let (running_node, shell_handle) = node.start().await?;
```

- **RunningNode**: Shell calls Workload (existing pattern)
- **ShellHandle**: Workload calls Shell (NEW capability)

### Symmetric Communication Pattern

```text
Shell (App)              Workload (Actor)
    │                            │
    ├─── RunningNode.call() ────→│  Shell → Workload
    │                            │
    │←─── ShellHandle.recv() ────┤  Workload → Shell
    │                            │
    ├─── ShellHandle.respond() →│  Shell responds
```

### Usage Example

```rust
// Shell main loop with bidirectional handling
loop {
    tokio::select! {
        // User input → Workload
        line = stdin.read_line() => {
            let resp = running_node.call(EchoRequest { message: line }).await?;
            println!("Reply: {}", resp.reply);
        }

        // Workload → Shell requests
        Some(request) = shell_handle.recv() => {
            match request.route_key() {
                "app.ShowNotification" => {
                    let notif: Notification = request.decode()?;
                    println!("📢 {}", notif.message);
                    shell_handle.respond(&request, b"ok".to_vec().into()).await?;
                }
                _ => {}
            }
        }
    }
}
```

### Future Plans

- **RPC Method Binding**: Type-safe proto-generated handlers for Shell side
- **Media Frame Support**: Real-time media streaming to Shell

## See Also

- [echo-real-server](../echo-real-server/) - Server implementation
- [shell-echo.sh](../../shell-echo.sh) - Full integration test script
- [ShellHandle Unit Tests](../../crates/runtime/tests/shell_handle_unit_test.rs) - ShellHandle test cases
