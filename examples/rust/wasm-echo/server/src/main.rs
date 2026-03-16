//! WASM Echo Server — host binary that loads a WASM echo actor
//!
//! Demonstrates the Actor-RTC WASM executor adapter pattern:
//! 1. Load pre-built asyncified WASM binary
//! 2. Compile & instantiate via WasmHost
//! 3. Register with AIS HTTP to obtain credential
//! 4. Attach a shell Workload + WASM executor to ActrSystem
//! 5. Inject credential and start ActrNode

mod shell_workload;

use shell_workload::ShellWorkload;

use actr_hyper::wasm::{WasmActorConfig, WasmHost};
use actr_hyper::{ActrSystem, AisClient, init_observability};
use actr_protocol::{RegisterRequest, register_response};
use std::path::PathBuf;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. Load configuration
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("🚀 WASM Echo Server starting");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📦 Loading WASM echo actor module");
    info!("📡 Signaling server: ws://localhost:8081/signaling/ws");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load pre-built asyncified WASM binary
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let wasm_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../guest/built/wasm_echo_guest.wasm");
    let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| {
        error!("❌ Failed to read WASM binary at {:?}: {}", wasm_path, e);
        error!("💡 Run the build step first: cd ../guest && ./build.sh");
        e
    })?;
    info!("📦 Loaded WASM binary: {} bytes", wasm_bytes.len());

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. Compile and instantiate WASM module
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🔧 Compiling WASM module...");
    let host = WasmHost::compile(&wasm_bytes)?;
    let mut instance = host.instantiate()?;

    // Initialize WASM actor with config
    let wasm_config = WasmActorConfig {
        actr_type: "acme:WasmEchoService:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&wasm_config)?;
    info!("✅ WASM instance initialized");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Register with AIS HTTP to obtain credential
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
    // 5. Create ActrSystem
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
    // 6. Attach shell workload + WASM executor, inject credential, then start
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 Attaching ShellWorkload with WASM executor...");
    let shell = ShellWorkload;
    let mut node = system.attach(shell).with_wasm_instance(instance);

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
    info!("🎉 WASM Echo Server fully started and registered");
    info!("📡 Waiting for client connections...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 7. Wait for Ctrl+C and shutdown
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ WASM Echo Server stopped");
    Ok(())
}
