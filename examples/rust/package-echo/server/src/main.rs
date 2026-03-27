//! Package Echo Host — host binary that loads the local echo-actr package
//!
//! Demonstrates the Actor-RTC package-driven workload loading pattern:
//! 1. Load a signed .actr package from the local echo-actr build output
//! 2. Let Hyper verify the package and build an ActrNode (attach)
//! 3. Register with AIS HTTP to obtain credential
//! 4. Inject credential and start ActrNode

use std::env;
use std::path::PathBuf;

use actr_hyper::{Hyper, HyperConfig, TrustMode, WorkloadPackage, init_observability};
use actr_platform_native::NativePlatformProvider;
use anyhow::{Context, Result, anyhow, ensure};
use base64::Engine;
use serde_json::Value;
use tracing::{error, info};

const DEFAULT_ECHO_ACTR_VERSION: &str = "0.2.1";

fn echo_actr_version() -> String {
    env::var("ECHO_ACTR_VERSION").unwrap_or_else(|_| DEFAULT_ECHO_ACTR_VERSION.to_string())
}

fn package_path() -> PathBuf {
    env::var("ACTR_PACKAGE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let echo_actr_version = echo_actr_version();
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!(
                    "../../../../../echo-actr/dist/actrium-EchoService-{echo_actr_version}-wasm32-unknown-unknown.actr"
                ))
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
    // 1. Load WorkloadPackage and extract manifest
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
    let package = WorkloadPackage::new(package_bytes.clone());

    let manifest_str = actr_pack::read_manifest_raw(&package_bytes)?;
    let manifest_raw: actr_config::RawConfig = toml::from_str(&manifest_str)?;
    let package_info = manifest_raw.package.into_package_info()?;
    
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load runtime configuration
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_actr_file(&config_path, package_info, vec![])?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("🚀 Package Echo Host starting");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📦 Loading local echo-actr package");
    info!("📡 Signaling server: ws://localhost:8081/signaling/ws");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let hyper_data_dir = config.config_dir.join(".hyper");

    // Determine trust mode: TRUST_MODE=production uses MFR cert cache (fetches keys from AIS),
    // otherwise use development mode with local self-signed public key
    let trust_mode = if env::var("TRUST_MODE")
        .map(|v| v == "production")
        .unwrap_or(false)
    {
        let ais_endpoint =
            env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
        // cert_cache needs base URL without /ais path suffix
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
        error!(
            "❌ Hyper initialization with NativePlatformProvider failed: {:?}",
            e
        );
    })?;
    info!(
        "✅ Hyper initialized with NativePlatformProvider, data_dir={}",
        hyper_data_dir.display()
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Attach package workload, inject credential, then start
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 Attaching package workload...");
    // Extract values needed for bootstrap_credential before config is moved into attach_package()
    let realm_id = config.realm.realm_id;
    let service_spec = config.calculate_service_spec();
    let acl = config.acl.clone();
    let mut node = hyper
        .attach_package(&package, config)
        .await
        .inspect_err(|e| error!("❌ hyper.attach_package failed: {:?}", e))?;

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Register with AIS HTTP to obtain credential
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        std::env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let register_ok = hyper
        .bootstrap_node_credential(&node, &ais_endpoint, realm_id, service_spec, acl)
        .await
        .inspect_err(|e| {
            error!("❌ AIS registration failed: {:?}", e);
        })?;
    info!(
        "✅ AIS registration successful, ActrId: {}",
        actr_protocol::ActrIdExt::to_string_repr(&register_ok.actr_id)
    );

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
