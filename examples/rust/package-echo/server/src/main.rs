//! Package Echo Host — host binary that loads the local echo-actr package
//!
//! Demonstrates the Actor-RTC package-driven workload loading pattern:
//! 1. Load a signed .actr package from the local echo-actr build output
//! 2. Let Hyper verify the package and prepare the workload
//! 3. Register with AIS HTTP to obtain credential
//! 4. Attach the package-selected workload to ActrSystem
//! 5. Inject credential and start ActrNode

use std::env;
use std::path::PathBuf;

use actr_hyper::{ActrSystem, Hyper, HyperConfig, TrustMode, WorkloadPackage, init_observability};
use actr_platform_native::NativePlatformProvider;
use anyhow::{Context, Result, anyhow, ensure};
use base64::Engine;
use serde_json::Value;
use tracing::{error, info};

fn package_path() -> PathBuf {
    env::var("ACTR_PACKAGE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../echo-actr/dist/actrium-EchoService-0.1.0-wasm32-unknown-unknown.actr")
        })
}

fn public_key_path() -> PathBuf {
    env::var("ACTR_PUBLIC_KEY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../echo-actr/public-key.json")
        })
}

fn load_package_public_key() -> Result<Vec<u8>> {
    let key_path = public_key_path();
    let value: Value =
        serde_json::from_reader(std::fs::File::open(&key_path).with_context(|| {
            format!(
                "Failed to read package public key at {}. Run `./start.sh` to build the local echo-actr package first.",
                key_path.display(),
            )
        })?)
        .context("Failed to parse package public key JSON")?;
    let public_key_b64 = value["public_key"]
        .as_str()
        .ok_or_else(|| anyhow!("package public key JSON missing `public_key` field"))?;
    let public_key = base64::engine::general_purpose::STANDARD.decode(public_key_b64)?;
    ensure!(
        public_key.len() == 32,
        "package public key `public_key` must be exactly 32 bytes"
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

    info!("🚀 Package Echo Host starting");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📦 Loading local echo-actr package");
    info!("📡 Signaling server: ws://localhost:8081/signaling/ws");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load WorkloadPackage and initialize Hyper
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let package_path = package_path();
    let package_bytes = std::fs::read(&package_path).inspect_err(|e| {
        error!(
            "❌ Failed to read WorkloadPackage at {:?}: {}",
            package_path, e
        );
        error!("💡 Run ./start.sh to build and verify the local echo-actr package first");
    })?;
    info!("📦 Loaded WorkloadPackage: {} bytes", package_bytes.len());
    let package = WorkloadPackage::new(package_bytes);

    let hyper_data_dir = config.config_dir.join(".hyper");
    let hyper = Hyper::init_with_platform(
        HyperConfig::new(&hyper_data_dir).with_trust_mode(TrustMode::Development {
            self_signed_pubkey: load_package_public_key()?,
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
        .load_workload_package(&package)
        .await
        .inspect_err(|e| {
            error!(
                "❌ WorkloadPackage verification or workload preparation failed at {}: {:?}",
                package_path.display(),
                e
            );
        })?;
    let package_manifest = loaded_package.manifest;
    info!(
        "✅ WorkloadPackage verified and workload prepared: {} ({:?})",
        package_manifest.actr_type_str(),
        loaded_package.backend
    );
    info!("✅ Package workload initialized");

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
            config.realm.realm_id,
            config.calculate_service_spec(),
            config.acl.clone(),
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
    // 5. Attach package workload, inject credential, then start
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 Attaching package-selected workload...");
    let mut node = system.attach_workload(loaded_package.workload);

    // Inject AIS-issued credential so start() populates signaling identity before connect
    node.inject_credential(register_ok);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await.inspect_err(|e| {
        error!("❌ ActrNode start failed: {:?}", e);
        error!("💡 Ensure signaling server is running: ws://localhost:8081");
    })?;

    info!("✅ ActrNode started successfully");
    info!(
        "🆔 Server ID: {}",
        actr_protocol::ActrIdExt::to_string_repr(actr_ref.actor_id())
    );
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 Package Echo Host fully started and registered");
    info!("📡 Waiting for client connections...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 6. Wait for Ctrl+C and shutdown
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Package Echo Host stopped");
    Ok(())
}
