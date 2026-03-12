//! Echo Server — Native / Process Mode Demo
//!
//! Demonstrates two execution modes for the same Workload:
//!
//! ## Native Mode (default)
//! ```bash
//! cargo run --bin echo-server -- --config actr-native.toml
//! ```
//! ActrSystem connects to the signaling server and registers on its own.
//!
//! ## Process Mode
//! ```bash
//! ACTR_REGISTER_OK=<base64> cargo run --bin echo-server -- --config actr-process.toml
//! ```
//! Hyper pre-registers with AIS and injects `RegisterOk` via the
//! `ACTR_REGISTER_OK` environment variable. ActrSystem skips signaling
//! registration and uses the injected credential directly.

mod echo_service;
mod generated;

use echo_service::EchoService;
use generated::echo_actor::EchoServiceWorkload;

use actr_hyper::prelude::*;
use std::path::PathBuf;
use tracing::{error, info};

fn print_banner(mode: &str) {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🚀 Echo Server — {} Mode Demo", mode);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. Determine config file (default: actr-native.toml) ──
    let config_file = std::env::args()
        .skip_while(|a| a != "--config")
        .nth(1)
        .unwrap_or_else(|| "actr-native.toml".to_string());

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&config_file);
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    let mode_str = format!("{}", config.execution_mode);
    print_banner(&mode_str);

    info!("📋 Execution mode: {}", config.execution_mode);
    info!("📋 Config file: {}", config_file);

    // ── 2. Create ActrSystem ──
    info!("🏗️  Creating ActrSystem...");
    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem created");

    // ── 3. Create EchoService and attach Workload ──
    info!("📦 Creating EchoService...");
    let echo_service = EchoService::new();
    let workload = EchoServiceWorkload::new(echo_service);
    let mut node = system.attach(workload);
    info!("✅ EchoService attached");

    // ── 4. Process Mode: inject credential from env var ──
    //
    // In Process mode, the Hyper host layer has already completed AIS
    // registration and passes the RegisterOk as a base64-encoded protobuf
    // via the ACTR_REGISTER_OK environment variable.
    if let Ok(register_ok_b64) = std::env::var("ACTR_REGISTER_OK") {
        info!("🔑 Found ACTR_REGISTER_OK env var, injecting credential...");

        use actr_protocol::prost::Message;
        use actr_protocol::register_response::RegisterOk;
        use base64::prelude::*;

        let bytes = BASE64_STANDARD
            .decode(&register_ok_b64)
            .map_err(|e| format!("Failed to decode ACTR_REGISTER_OK base64: {e}"))?;
        let register_ok = RegisterOk::decode(bytes.as_slice())
            .map_err(|e| format!("Failed to decode RegisterOk protobuf: {e}"))?;

        info!(
            "🔑 Injecting pre-issued credential (ActrId: {:?})",
            register_ok.actr_id
        );
        node.inject_credential(register_ok);
        info!("✅ Credential injected, start() will skip signaling registration");
    }

    // ── 5. Start ActrNode ──
    info!("🚀 Starting ActrNode...");
    let actr_ref = match node.start().await {
        Ok(actr) => actr,
        Err(e) => {
            error!("❌ ActrNode start failed: {:?}", e);
            if mode_str == "native" {
                error!("💡 Hint: ensure signaling server (Actrix) is running on ws://localhost:8081");
            } else {
                error!("💡 Hint: in Process mode, ensure ACTR_REGISTER_OK env var contains valid RegisterOk");
            }
            return Err(e.into());
        }
    };

    info!("✅ ActrNode started!");
    info!("🆔 Server ID: {:?}", actr_ref.actor_id());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎉 Echo Server running in {} mode", mode_str);
    println!("📡 Waiting for client connections...");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ── 6. Wait for Ctrl+C ──
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;
    info!("✅ Echo Server shut down");

    Ok(())
}
