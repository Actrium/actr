//! WASM Echo Server — host binary that loads a signed .actr package
//!
//! Demonstrates the full Actor-RTC package verification and credential bootstrap flow:
//! 1. Load a signed `.actr` package (built by `actr pkg build`)
//! 2. Verify package signature using the MFR public key
//! 3. Register with AIS using the verified manifest + MFR signature
//! 4. Compile & instantiate the WASM binary from the verified package
//! 5. Attach a shell Workload + WASM executor to ActrSystem
//! 6. Inject credential and start ActrNode

mod shell_workload;

use shell_workload::ShellWorkload;

use actr_hyper::wasm::{WasmActorConfig, WasmHost};
use actr_hyper::{ActrSystem, AisClient, Hyper, init_observability};
use actr_protocol::{ActrType, Realm, RegisterRequest, register_response};
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

    info!("🚀 WASM Echo Server starting (full package verification flow)");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Load and verify the signed .actr package
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let actr_pkg_path = std::env::var("ACTR_PACKAGE_PATH").unwrap_or_else(|_| {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .join("../guest/built/wasm-echo.actr")
            .to_string_lossy()
            .to_string()
    });
    let actr_pkg_path = PathBuf::from(&actr_pkg_path);

    info!("📦 Loading .actr package: {:?}", actr_pkg_path);
    let package_bytes = std::fs::read(&actr_pkg_path).map_err(|e| {
        error!(
            "❌ Failed to read .actr package at {:?}: {}",
            actr_pkg_path, e
        );
        error!("💡 Build the package first: actr pkg build -b <wasm> -c actr.toml -k <key>");
        e
    })?;
    info!("📦 Package loaded: {} bytes", package_bytes.len());

    // Verify package signature using MFR public key (set by start.sh from keygen output)
    let mfr_pubkey = std::env::var("MFR_PUBKEY")
        .expect("MFR_PUBKEY environment variable is required (base64-encoded Ed25519 public key)");

    info!("🔐 Verifying package signature...");
    let hyper = {
        use actr_hyper::config::{HyperConfig, TrustMode};
        use base64::Engine;

        let pubkey_bytes = base64::engine::general_purpose::STANDARD
            .decode(&mfr_pubkey)
            .expect("Invalid MFR_PUBKEY base64");

        let hyper_config = HyperConfig::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"))
            .with_trust_mode(TrustMode::Development {
                self_signed_pubkey: pubkey_bytes,
            });

        Hyper::init(hyper_config).await?
    };

    let manifest = hyper.verify_package(&package_bytes).await?;
    info!(
        "✅ Package verified: {}:{}:{}",
        manifest.manufacturer, manifest.actr_name, manifest.version
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. Register with AIS using verified manifest + MFR signature
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        std::env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let ais = AisClient::new(&ais_endpoint);
    let register_req = RegisterRequest {
        actr_type: ActrType {
            manufacturer: manifest.manufacturer.clone(),
            name: manifest.actr_name.clone(),
            version: manifest.version.clone(),
        },
        realm: Realm {
            realm_id: config.realm.realm_id,
        },
        service_spec: config.calculate_service_spec(),
        acl: config.acl.clone(),
        service: None,
        ws_address: None,
        manifest_raw: Some(manifest.manifest_raw.clone().into()),
        mfr_signature: Some(manifest.signature.clone().into()),
        psk_token: None,
        target: Some(manifest.target.clone()),
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
    // 4. Extract and compile WASM binary from the verified package
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🔧 Extracting and compiling WASM from verified package...");
    let wasm_bytes = actr_pack::load_binary(&package_bytes).map_err(|e| {
        error!("❌ Failed to extract binary from .actr package: {}", e);
        e
    })?;
    info!("📦 WASM binary extracted: {} bytes", wasm_bytes.len());

    let host = WasmHost::compile(&wasm_bytes)?;
    let mut instance = host.instantiate()?;

    // Initialize WASM actor with config from verified manifest
    let wasm_config = WasmActorConfig {
        actr_type: manifest.actr_type_str().to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&wasm_config)?;
    info!("✅ WASM instance initialized");

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
