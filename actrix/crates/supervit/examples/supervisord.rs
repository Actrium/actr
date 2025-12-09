//! Example: Supervisord gRPC Server
//!
//! This example demonstrates how to run a standalone Supervisord gRPC server
//! that can be tested with grpcurl.
//!
//! # Usage
//!
//! 1. Run the server:
//!    ```bash
//!    cargo run -p supervit --example supervisord
//!    ```
//!
//! 2. Test with the provided shell script (recommended):
//!    ```bash
//!    ./crates/supervit/scripts/test_supervised.sh list
//!    ./crates/supervit/scripts/test_supervised.sh node_info
//!    ./crates/supervit/scripts/test_supervised.sh list_realms
//!    ./crates/supervit/scripts/test_supervised.sh create_realm --realm-id my-realm
//!    ```
//!
//! 3. Or test manually with grpcurl (list services):
//!    ```bash
//!    grpcurl -plaintext -import-path crates/actrix-proto/proto -proto supervised.proto -proto common.proto localhost:50055 list
//!    ```
//!
//! # Note
//!
//! This example uses SqliteNonceStorage for nonce storage.
//! A temporary database is created in the system temp directory.

use std::sync::Arc;

use actrix_common::ServiceCollector;
use actrix_common::storage::{SqliteNonceStorage, db::set_db_path};
use hex;
use tonic::transport::Server;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

use supervit::{AuthService, SupervisedServiceServer, Supervisord};

/// Shared secret for testing (hex encoded, 32 bytes)
const TEST_SHARED_SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Create temporary directory for storage
    let temp_dir = std::env::temp_dir().join("supervit_example");
    std::fs::create_dir_all(&temp_dir)?;

    info!("Using temp directory: {:?}", temp_dir);

    // Initialize main database (required for realm operations)
    set_db_path(&temp_dir).await?;
    info!("Main database initialized");

    // Create nonce storage
    let nonce_storage = Arc::new(SqliteNonceStorage::new_async(&temp_dir).await?);
    info!("Nonce storage initialized");

    // Create empty service collector for demo
    let service_collector = ServiceCollector::new();

    // Create supervisord service
    let service = Supervisord::new(
        "example-node-01",
        "Example Node",
        "local,dev,example",
        env!("CARGO_PKG_VERSION"),
        service_collector,
    )?;

    // Allow overriding bind address for sandboxed environments (default: 127.0.0.1:50055)
    let bind = std::env::var("SUPERVISORD_BIND").unwrap_or_else(|_| "127.0.0.1:50055".to_string());
    let addr = bind.parse()?;
    info!("🚀 Supervisord gRPC server listening on {}", addr);
    info!("");
    info!("Test with shell script (recommended, run from project root):");
    info!("  ./crates/supervit/scripts/test_supervised.sh list");
    info!("  ./crates/supervit/scripts/test_supervised.sh node_info");
    info!("  ./crates/supervit/scripts/test_supervised.sh list_realms");
    info!("  ./crates/supervit/scripts/test_supervised.sh create_realm --realm-id test-realm-01");
    info!("");
    info!("Or test with grpcurl manually:");
    info!(
        "  grpcurl -plaintext -import-path crates/actrix-proto/proto -proto supervised.proto -proto common.proto localhost:50055 list"
    );
    info!("");
    info!("Shared secret (for testing): {}", TEST_SHARED_SECRET);

    let authed_service = AuthService::new(
        service,
        "example-node-01",
        Arc::new(hex::decode(TEST_SHARED_SECRET)?),
        nonce_storage,
        300,
    );

    Server::builder()
        .add_service(SupervisedServiceServer::new(authed_service))
        .serve(addr)
        .await?;

    Ok(())
}
