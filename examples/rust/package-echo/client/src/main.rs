//! Package Echo Client Host — package-backed host that loads the client-guest .actr package.
//!
//! Mirrors the server host pattern:
//! 1. Load signed client-guest .actr package
//! 2. Hyper verifies and attaches the package workload
//! 3. Register with AIS to obtain credential
//! 4. Start ActrNode
//! 5. Read stdin and dispatch each line to the local guest via actr_ref.call()

/// Generated protobuf types from client.proto
pub mod client {
    include!(concat!(env!("OUT_DIR"), "/client.rs"));
}

use std::env;
use std::path::PathBuf;

use actr_hyper::{Hyper, HyperConfig, TrustMode, WorkloadPackage, init_observability};
use actr_platform_native::NativePlatformProvider;
use actr_protocol::RpcRequest;
use anyhow::{Context, Result, anyhow, ensure};
use base64::Engine;
use serde_json::Value;
use tracing::{error, info};

use crate::client::{SendMessageRequest, SendMessageResponse};

impl RpcRequest for SendMessageRequest {
    type Response = SendMessageResponse;

    fn route_key() -> &'static str {
        "client.ClientService.SendMessage"
    }

    fn payload_type() -> actr_protocol::PayloadType {
        actr_protocol::PayloadType::RpcReliable
    }
}

fn package_path() -> PathBuf {
    env::var("CLIENT_GUEST_PACKAGE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../client-guest/dist/acme-package-echo-client-guest-0.1.0-cdylib.actr")
        })
}

fn public_key_path() -> PathBuf {
    env::var("CLIENT_GUEST_PUBLIC_KEY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../client-guest/public-key.json")
        })
}

fn load_package_public_key() -> Result<Vec<u8>> {
    let key_path = public_key_path();
    let value: Value =
        serde_json::from_reader(std::fs::File::open(&key_path).with_context(|| {
            format!(
                "Failed to read client-guest public key at {}. Run `./start.sh` to build first.",
                key_path.display(),
            )
        })?)
        .context("Failed to parse client-guest public key JSON")?;
    let public_key_b64 = value["public_key"]
        .as_str()
        .ok_or_else(|| anyhow!("client-guest public key JSON missing `public_key` field"))?;
    let public_key = base64::engine::general_purpose::STANDARD.decode(public_key_b64)?;
    ensure!(
        public_key.len() == 32,
        "client-guest public key must be exactly 32 bytes"
    );
    Ok(public_key)
}

#[tokio::main]
async fn main() -> Result<()> {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. Load configuration
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("🚀 Package Echo Client Host starting");
    info!("📡 Signaling server: {}", config.signaling_url);

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load WorkloadPackage and initialize Hyper
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let package_path = package_path();
    let package_bytes = std::fs::read(&package_path).inspect_err(|e| {
        error!(
            "❌ Failed to read client-guest package at {:?}: {}",
            package_path, e
        );
        error!("💡 Run ./start.sh to build the client-guest package first");
    })?;
    info!(
        "📦 Loaded client-guest package: {} bytes",
        package_bytes.len()
    );
    let package = WorkloadPackage::new(package_bytes);

    let hyper_data_dir = config.config_dir.join(".hyper");

    let trust_mode = if env::var("TRUST_MODE")
        .map(|v| v == "production")
        .unwrap_or(false)
    {
        let ais_endpoint =
            env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
        let base_endpoint = ais_endpoint.trim_end_matches("/ais").to_string();
        info!(
            "🔐 Using Production trust mode (base endpoint: {})",
            base_endpoint
        );
        TrustMode::Production {
            ais_endpoint: base_endpoint,
        }
    } else {
        info!("🔐 Using Development trust mode (local public key)");
        TrustMode::Development {
            self_signed_pubkey: load_package_public_key()?,
        }
    };

    let hyper = Hyper::init_with_platform(
        HyperConfig::new(&hyper_data_dir).with_trust_mode(trust_mode),
        std::sync::Arc::new(NativePlatformProvider::new()),
    )
    .await
    .inspect_err(|e| {
        error!("❌ Hyper initialization failed: {:?}", e);
    })?;
    info!(
        "✅ Hyper initialized, data_dir={}",
        hyper_data_dir.display()
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. Attach package workload
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 Attaching client-guest package workload...");
    let realm_id = config.realm.realm_id;
    let service_spec = config.calculate_service_spec();
    let acl = config.acl.clone();
    let mut node = hyper
        .attach_package(&package, config)
        .await
        .inspect_err(|e| error!("❌ hyper.attach_package failed: {:?}", e))?;

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Register with AIS
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let register_ok = hyper
        .bootstrap_node_credential(&node, &ais_endpoint, realm_id, service_spec, acl)
        .await
        .inspect_err(|e| error!("❌ AIS registration failed: {:?}", e))?;
    info!(
        "✅ AIS registration successful, ActrId: {}",
        actr_protocol::ActrIdExt::to_string_repr(&register_ok.actr_id)
    );

    node.inject_credential(register_ok);

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Start ActrNode
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await.inspect_err(|e| {
        error!("❌ ActrNode start failed: {:?}", e);
    })?;
    info!(
        "✅ ActrNode started with ID: {}",
        actr_protocol::ActrIdExt::to_string_repr(actr_ref.actor_id())
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 6. Interactive stdin loop — dispatch to local guest
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("===== Package Echo Client =====");
    println!("Type messages to send to the echo server (type 'quit' to exit):");

    use std::io::Write;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();

    print!("> ");
    std::io::stdout().flush().unwrap();

    while let Ok(Some(line)) = reader.next_line().await {
        let line = line.trim().to_string();

        if line == "quit" || line == "exit" {
            info!("[App] User requested exit");
            break;
        }

        if line.is_empty() {
            print!("> ");
            std::io::stdout().flush().unwrap();
            continue;
        }

        info!("[App] Dispatching to local guest: {}", line);

        let request = SendMessageRequest {
            message: line.clone(),
        };

        match actr_ref.call(request).await {
            Ok(response) => {
                let response: SendMessageResponse = response;
                println!("\n[Received reply] {}", response.reply);
            }
            Err(e) => {
                error!("[App] Guest dispatch failed: {:?}", e);
                println!("\n[Error] {}", e);
            }
        }

        print!("> ");
        std::io::stdout().flush().unwrap();
    }

    actr_ref.shutdown();
    actr_ref.wait_for_shutdown().await;

    info!("👋 Client shut down");
    Ok(())
}
