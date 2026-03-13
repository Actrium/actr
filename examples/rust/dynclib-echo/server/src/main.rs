//! Dynclib Echo Server — host binary that loads a native shared library echo actor
//!
//! Demonstrates the Actor-RTC dynclib executor adapter pattern:
//! 1. Build guest actor as a native cdylib (.dylib/.so)
//! 2. Load via DynclibHost
//! 3. Register with AIS HTTP to obtain credential
//! 4. Attach a shell Workload + dynclib executor to ActrSystem
//! 5. Inject credential and start ActrNode

mod shell_workload;

use shell_workload::ShellWorkload;

use actr_hyper::dynclib::DynclibHost;
use actr_hyper::{ActrSystem, AisClient, init_observability};
use actr_protocol::{RegisterRequest, register_response};
use std::path::PathBuf;
use tracing::{error, info};

/// Determine the cdylib filename based on the current platform
fn cdylib_filename(name: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{}.dylib", name)
    } else if cfg!(target_os = "windows") {
        format!("{}.dll", name)
    } else {
        format!("lib{}.so", name)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. Load configuration
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("🚀 Dynclib Echo Server starting");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📦 Loading native shared library echo actor");
    info!("📡 Signaling server: ws://localhost:8081/signaling/ws");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load native shared library
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let lib_name = cdylib_filename("dynclib_echo_guest");
    let lib_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../guest/target/release")
        .join(&lib_name);
    info!("📦 Loading shared library: {}", lib_path.display());

    let host = DynclibHost::load(&lib_path).map_err(|e| {
        error!("❌ Failed to load shared library at {:?}: {}", lib_path, e);
        error!("💡 Build the guest first: cd ../guest && cargo build --release");
        e
    })?;

    // Instantiate actor (empty config)
    let instance = host.instantiate(b"{}").map_err(|e| {
        error!("❌ Failed to instantiate dynclib actor: {}", e);
        e
    })?;
    info!("✅ Dynclib instance initialized");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. Register with AIS HTTP to obtain credential
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        std::env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let ais = AisClient::new(&ais_endpoint);
    let register_req = RegisterRequest {
        actr_type: config.actr_type().clone(),
        realm: config.realm.clone(),
        service_spec: config.calculate_service_spec(),
        acl: config.acl.clone(),
        service: None,
        ws_address: None,
        manifest_json: None,
        mfr_signature: None,
        psk_token: None,
    };

    let ais_response = ais
        .register_with_manifest(register_req)
        .await
        .map_err(|e| {
            error!("❌ AIS registration failed: {:?}", e);
            e
        })?;

    let register_ok = match ais_response.result {
        Some(register_response::Result::Success(ok)) => {
            info!(
                "✅ AIS registration successful, ActrId: {}",
                actr_protocol::ActrIdExt::to_string_repr(&ok.actr_id)
            );
            ok
        }
        Some(register_response::Result::Error(e)) => {
            error!(
                "❌ AIS registration rejected: code={}, message={}",
                e.code, e.message
            );
            return Err(format!("AIS rejected: {} (code={})", e.message, e.code).into());
        }
        None => {
            error!("❌ AIS response missing result");
            return Err("AIS response missing result".into());
        }
    };

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Create ActrSystem
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🏗️  Creating ActrSystem...");
    let system = match ActrSystem::new(config).await {
        Ok(sys) => sys,
        Err(e) => {
            error!("❌ ActrSystem creation failed: {:?}", e);
            return Err(e.into());
        }
    };
    info!("✅ ActrSystem created");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Attach shell workload + dynclib executor, inject credential, then start
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 Attaching ShellWorkload with dynclib executor...");
    let shell = ShellWorkload;
    let mut node = system.attach(shell).with_executor(Box::new(instance));

    // Inject AIS-issued credential so start() populates signaling identity before connect
    node.inject_credential(register_ok);

    info!("🚀 Starting ActrNode...");
    let actr_ref = match node.start().await {
        Ok(r) => r,
        Err(e) => {
            error!("❌ ActrNode start failed: {:?}", e);
            error!("💡 Ensure signaling server is running: ws://localhost:8081");
            return Err(e.into());
        }
    };

    info!("✅ ActrNode started successfully");
    info!("🆔 Server ID: {:?}", actr_ref.actor_id());
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 Dynclib Echo Server fully started and registered");
    info!("📡 Waiting for client connections...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 6. Wait for Ctrl+C and shutdown
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Dynclib Echo Server stopped");
    Ok(())
}
