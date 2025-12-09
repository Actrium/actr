# Supervit - gRPC Supervisor Client/Server

A high-performance gRPC client and supervisord server for connecting actrix nodes to the centralized actrix-supervisor management platform.

## Features

- **gRPC Communication**: Uses HTTP/2 and Protocol Buffers for efficient communication
- **Status Reporting**: Automatic periodic system metrics and service status reporting
- **Configuration Management**: Receive and apply configuration updates from supervisor
- **Realm Operations**: Remote realm CRUD operations, stored via common `Realm` model
- **Health Checks**: Built-in health check and heartbeat mechanism
- **Supervisord Service**: Built-in `SupervisedService` implementation for realm delivery and node control

## Architecture

```
┌─────────────────┐           gRPC/HTTP2           ┌─────────────────┐
│  actrix-node    │◄────────────────────────────►│ actrix-supervisor│
│  (SupervitClient)│         Unary RPC            │  (gRPC Server)  │
└─────────────────┘                               └─────────────────┘
```

## Configuration

Add to your `config.toml`:

```toml
[supervisor]
connect_timeout_secs = 30
status_report_interval_secs = 60
health_check_interval_secs = 30
enable_tls = false

[supervisor.supervisord]
node_name = "actrix-node"
ip = "0.0.0.0"
port = 50055
advertised_ip = "127.0.0.1"

[supervisor.client]
node_id = "actrix-node-01"
endpoint = "http://supervisor.example.com:50051"
shared_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
```

For TLS connections:

```toml
[supervisor]
enable_tls = true
tls_domain = "supervisor.example.com"

[supervisor.supervisord]
node_name = "actrix-node"
ip = "0.0.0.0"
port = 50055
advertised_ip = "127.0.0.1"

[supervisor.client]
node_id = "actrix-node-01"
endpoint = "https://supervisor.example.com:50051"
shared_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
```

## Usage

### Client

```rust
use supervit::{SupervitClient, SupervitConfig};
use actrix_common::ServiceCollector;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create configuration
    let config = SupervitConfig {
        node_id: "actrix-01".to_string(),
        endpoint: "http://localhost:50051".to_string(),
        shared_secret: Some(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        ),
        ..Default::default()
    };

    // Create service collector (required for client)
    let service_collector = ServiceCollector::new();

    // Create and connect client
    let mut client = SupervitClient::new(config, service_collector)?;
    client.connect().await?;

    // Start automatic status reporting
    client.start_status_reporting().await?;

    // Perform health check
    let health = client.health_check().await?;
    println!("Health check: {:?}", health);

    Ok(())
}
```

### Supervisord service (server)

```rust
use supervit::{Supervisord, AuthService, SupervisedServiceServer};
use actrix_common::{ServiceCollector, storage::SqliteNonceStorage};
use std::sync::Arc;
use hex;

// Initialize database and nonce storage
let nonce_storage = Arc::new(SqliteNonceStorage::new_async("/var/lib/actrix").await?);
let service_collector = ServiceCollector::new();

// Create supervisord service
let service = Supervisord::new(
    "node-1",
    "actrix-node",
    "hangzhou",
    env!("CARGO_PKG_VERSION"),
    service_collector,
)?;

// Wrap with authentication layer
let shared_secret = hex::decode(
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
)?;
let authed_service = AuthService::new(
    service,
    "node-1",
    Arc::new(shared_secret),
    nonce_storage,
    300, // max_clock_skew_secs
);

// Use with tonic Server
// Server::builder()
//     .add_service(SupervisedServiceServer::new(authed_service))
//     .serve(addr)
//     .await?;
```

## Protocol

The communication protocol is defined in `proto/supervisor.proto` (SupervisorService) and `proto/supervised.proto` (SupervisedService).

### Services

- **SupervisorService**: `RegisterNode`, `Report`, `HealthCheck`
- **SupervisedService**: `UpdateConfig`, `GetConfig`, realm CRUD (`CreateRealm`, `GetRealm`, `UpdateRealm`, `DeleteRealm`, `ListRealms`), `GetNodeInfo`, `Shutdown`

### Message Types

- `RegisterNodeRequest/Response`: Node registration handshake
- `ReportRequest` / `ReportResponse`: System metrics and service status reporting
- `UpdateConfigRequest/Response`, `GetConfigRequest/Response`: Configuration management
- `CreateRealmRequest/Response`, `GetRealmRequest/Response`, `UpdateRealmRequest/Response`, `DeleteRealmRequest/Response`, `ListRealmsRequest/Response`: Realm CRUD
- `GetNodeInfoRequest/Response`, `ShutdownRequest/Response`: Node control
- `HealthCheckRequest/Response`: Health checks

## Building

The protocol is automatically compiled during build using `tonic-build`:

```bash
cargo build -p supervit
```

Generated code will be in `src/generated/`.

## Testing

```bash
cargo test -p supervit
```

## Comparison with WebSocket

| Feature          | gRPC                     | WebSocket (Old) |
| ---------------- | ------------------------ | --------------- |
| Code Generation  | ✅ Automatic              | ❌ Manual        |
| Type Safety      | ✅ Compile-time           | ⚠️ Runtime       |
| Monitoring       | ✅ Built-in               | ❌ Custom        |
| Load Balancing   | ✅ Native                 | ⚠️ Complex       |
| Debugging Tools  | ✅ Rich (grpcurl, grpcui) | ⚠️ Limited       |
| Development Time | ⚡ Fast                   | 🐢 Slow          |

## License

Apache 2.0
