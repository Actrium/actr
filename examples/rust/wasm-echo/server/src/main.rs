//! WASM Echo Server — host binary that loads a WASM echo actor
//!
//! Demonstrates the Actor-RTC package-driven executor loading pattern:
//! 1. Load pre-built asyncified WASM binary
//! 2. Let Hyper verify the package and prepare the executor
//! 3. Register with AIS HTTP to obtain credential
//! 4. Attach a shell Workload + package-selected executor to ActrSystem
//! 5. Inject credential and start ActrNode

mod shell_workload;

use std::path::PathBuf;

use actr_hyper::{
    ActrSystem, CredentialBootstrapRequest, Hyper, HyperConfig, TrustMode, init_observability,
};
use actr_platform_native::NativePlatformProvider;
use anyhow::{Context, Result, anyhow, ensure};
use base64::Engine;
use serde_json::Value;
use shell_workload::ShellWorkload;
use tracing::{error, info};

fn load_dev_public_key() -> Result<Vec<u8>> {
    let key_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dev-key.json");
    let value: Value =
        serde_json::from_reader(std::fs::File::open(&key_path).with_context(|| {
            format!(
                "Failed to read dev key at {}. Run `actr pkg keygen --output {}` first.",
                key_path.display(),
                key_path.display()
            )
        })?)
        .context("Failed to parse dev key JSON")?;
    let public_key_b64 = value["public_key"]
        .as_str()
        .ok_or_else(|| anyhow!("dev key JSON missing `public_key` field"))?;
    let public_key = base64::engine::general_purpose::STANDARD.decode(public_key_b64)?;
    ensure!(
        public_key.len() == 32,
        "dev key `public_key` must be exactly 32 bytes"
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

    info!("🚀 WASM Echo Server starting");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📦 Loading ActrPackage for WASM echo actor");
    info!("📡 Signaling server: ws://localhost:8081/signaling/ws");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load ActrPackage and initialize Hyper
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let package_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../guest/built/wasm_echo_guest.actr");
    let package_bytes = std::fs::read(&package_path).inspect_err(|e| {
        error!("❌ Failed to read ActrPackage at {:?}: {}", package_path, e);
        error!("💡 Run ./start.sh to build and package the guest actor first");
    })?;
    info!("📦 Loaded ActrPackage: {} bytes", package_bytes.len());

    let hyper_data_dir = config.config_dir.join(".hyper");
    let hyper = Hyper::init_with_platform(
        HyperConfig::new(&hyper_data_dir).with_trust_mode(TrustMode::Development {
            self_signed_pubkey: load_dev_public_key()?,
        }),
        std::sync::Arc::new(NativePlatformProvider::new()),
    )
    .await
    .inspect_err(|e| {
        error!(
            "❌ Hyper initialization with NativePlatformProvider failed: {:?}",
            e
        );
    })?;
    info!(
        "✅ Hyper initialized with NativePlatformProvider, data_dir={}",
        hyper_data_dir.display()
    );

    let loaded_package = hyper
        .load_package_executor(&package_bytes)
        .await
        .inspect_err(|e| {
            error!(
                "❌ ActrPackage verification or executor preparation failed at {}: {:?}",
                package_path.display(),
                e
            );
        })?;
    let package_manifest = loaded_package.manifest;
    info!(
        "✅ ActrPackage verified and executor prepared: {} ({:?})",
        package_manifest.actr_type_str(),
        loaded_package.backend
    );
    info!("✅ Package executor initialized");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. Register with AIS HTTP to obtain credential
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        std::env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let register_ok = hyper
        .bootstrap_credential(
            &package_manifest,
            &ais_endpoint,
            CredentialBootstrapRequest {
                realm: config.realm.clone(),
                service_spec: config.calculate_service_spec(),
                acl: config.acl.clone(),
                service: None,
                ws_address: None,
            },
        )
        .await
        .inspect_err(|e| {
            error!("❌ AIS registration failed: {:?}", e);
        })?;
    info!(
        "✅ AIS registration successful, ActrId: {}",
        actr_protocol::ActrIdExt::to_string_repr(&register_ok.actr_id)
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Create ActrSystem
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🏗️  Creating ActrSystem...");
    let system = ActrSystem::new(config).await.inspect_err(|e| {
        error!("❌ ActrSystem creation failed: {:?}", e);
    })?;
    info!("✅ ActrSystem created");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Attach shell workload + package executor, inject credential, then start
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 Attaching ShellWorkload with package-selected executor...");
    let shell = ShellWorkload;
    let mut node = system.attach(shell).with_executor(loaded_package.executor);

    // Inject AIS-issued credential so start() populates signaling identity before connect
    node.inject_credential(register_ok);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await.inspect_err(|e| {
        error!("❌ ActrNode start failed: {:?}", e);
        error!("💡 Ensure signaling server is running: ws://localhost:8081");
    })?;

    info!("✅ ActrNode started successfully");
    info!("🆔 Server ID: {:?}", actr_ref.actor_id());
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 WASM Echo Server fully started and registered");
    info!("📡 Waiting for client connections...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 6. Wait for Ctrl+C and shutdown
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ WASM Echo Server stopped");
    Ok(())
}
