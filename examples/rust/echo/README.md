# Echo Demo

This example demonstrates a basic Echo client and server communicating over the Actr network.

## Architecture

The Echo Demo consists of two separate Actr nodes:
1. **EchoServer**: Listens for incoming Remote Procedure Calls (RPC).
2. **EchoClient**: Discovers the server via the signaling server and sends messages to it.

Both nodes connect to an Actrix signaling server, discover each other, and establish a direct WebRTC peer-to-peer connection for communication.

## Quick Start

You can run the entire demo end-to-end using the provided `start.sh` script. This script will automatically build the client, server, and the Actrix signaling server, initialize the required SQLite database, and launch all components.

```bash
./start.sh
```

## File Structure

```text
echo/
├── proto/echo.proto           # Shared proto definition
├── server/
│   ├── Cargo.toml
│   ├── actr.toml              # Server configuration
│   └── src/
│       ├── main.rs            # Server entry point
│       └── echo_service.rs    # Business logic for echoing messages
├── client/
│   ├── Cargo.toml
│   ├── actr.toml              # Client configuration
│   └── src/
│       ├── main.rs            # Client entry point
│       ├── client_workload.rs # Client-side Actr workload
│       └── app_side.rs        # Interactive command-line loop
├── start.sh                   # Launch script
└── README.md                  # This file
```

## Manual Execution (Optional)

If you prefer to run the components manually without the `start.sh` script, follow these steps in separate terminal windows:

**1. Start Actrix Signaling Server**
```bash
cd ../../../../actrix
cargo run --bin actrix -- --config=config.example.toml
```

**2. Initialize Realm in Database**
```bash
sqlite3 ../../../../actrix/database/actrix.db "INSERT OR IGNORE INTO realm (id, name, status, enabled, created_at, secret_current) VALUES (33554432, 'Echo Realm', 'Active', 1, strftime('%s', 'now'), '');"
```

**3. Run Echo Server**
```bash
cd server
cargo run --bin echo-server -- --config=actr.toml
```

**4. Run Echo Client**
```bash
cd client
cargo run --bin echo-client -- --config=actr.toml "Hello from terminal!"
```
