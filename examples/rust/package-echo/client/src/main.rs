//! Package Echo Client Host — package-backed host that loads the client-guest .actr package.
//!
//! Mirrors the server host pattern:
//! 1. Load signed client-guest .actr package
//! 2. Hyper verifies and attaches the package workload
//! 3. Register with AIS to obtain credential
//! 4. Start ActrNode
//! 5. Read stdin and dispatch each line to the local guest via actr_ref.call()

/// Generated protobuf types from echo.proto
pub mod echo {
    include!(concat!(env!("OUT_DIR"), "/echo.rs"));
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

use crate::echo::{EchoRequest, EchoResponse};

impl RpcRequest for EchoRequest {
    type Response = EchoResponse;

    fn route_key() -> &'static str {
        "echo.EchoService.Echo"
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
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../client-guest/dist/public-key.json")
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
    // 1. Load WorkloadPackage and extract manifest
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
    let package = WorkloadPackage::new(package_bytes.clone());

    // Parse the PackageManifest from inside .actr (flat structure, no [package] section)
    let manifest = actr_pack::read_manifest(&package_bytes)?;
    let package_info = actr_config::PackageInfo {
        name: manifest.name.clone(),
        actr_type: actr_protocol::ActrType {
            manufacturer: manifest.manufacturer.clone(),
            name: manifest.name,
            version: manifest.version,
        },
        description: manifest.metadata.description,
        authors: vec![],
        license: manifest.metadata.license,
    };

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load runtime configuration
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_runtime_file(&config_path, package_info, vec![])?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("🚀 Package Echo Client Host starting");
    info!("📡 Signaling server: {:?}", config.signaling_url);

    let hyper_data_dir = actr_config::user_config::resolve_hyper_data_dir()?;

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

    let hyper = Hyper::with_platform(
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
    let attached = hyper
        .attach(&package, config)
        .await
        .inspect_err(|e| error!("❌ hyper.attach failed: {:?}", e))?;

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Register with AIS
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let registered = attached
        .register(&ais_endpoint)
        .await
        .inspect_err(|e| error!("❌ AIS registration failed: {:?}", e))?;
    info!("✅ AIS registration successful");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Start ActrNode
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🚀 Starting ActrNode...");
    let actr_ref = registered.start().await.inspect_err(|e| {
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

        match actr_ref
            .call(EchoRequest {
                message: line.clone(),
            })
            .await
        {
            Ok(response) => {
                let response: EchoResponse = response;
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
