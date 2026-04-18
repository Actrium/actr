//! Package Runtime Echo Client Host.
//!
//! Loads a local client guest package, registers with AIS, then dispatches
//! stdin lines to the guest which proxies requests to the remote EchoService.

pub mod echo {
    include!(concat!(env!("OUT_DIR"), "/echo.rs"));
}

use std::env;
use std::path::PathBuf;

use actr_hyper::{
    Hyper, HyperConfig, RegistryTrust, StaticTrust, TrustProvider, WorkloadPackage,
    init_observability,
};
use std::sync::Arc;
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
                .join("../client-guest/dist/actrium-pkg-runtime-echo-client-guest-0.1.0-cdylib.actr")
        })
}

fn public_key_path() -> PathBuf {
    env::var("CLIENT_GUEST_PUBLIC_KEY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../client-guest/dist/public-key.json")
        })
}

fn runtime_config_path() -> PathBuf {
    env::var("CLIENT_RUNTIME_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml"))
}

fn load_package_public_key() -> Result<Vec<u8>> {
    let key_path = public_key_path();
    let value: Value =
        serde_json::from_reader(std::fs::File::open(&key_path).with_context(|| {
            format!(
                "Failed to read client guest public key at {}. Run `bash e2e/package-runtime-echo/run.sh` first.",
                key_path.display(),
            )
        })?)
        .context("Failed to parse client guest public key JSON")?;
    let public_key_b64 = value["public_key"]
        .as_str()
        .ok_or_else(|| anyhow!("client guest public key JSON missing `public_key` field"))?;
    let public_key = base64::engine::general_purpose::STANDARD.decode(public_key_b64)?;
    ensure!(
        public_key.len() == 32,
        "client guest public key must be exactly 32 bytes"
    );
    Ok(public_key)
}

#[tokio::main]
async fn main() -> Result<()> {
    let package_path = package_path();
    let package_bytes = std::fs::read(&package_path).inspect_err(|e| {
        error!(
            "Failed to read client guest package at {:?}: {}",
            package_path, e
        );
        error!("Run `bash e2e/package-runtime-echo/run.sh` to build the package first");
    })?;
    info!("Loaded client guest package: {} bytes", package_bytes.len());
    let package = WorkloadPackage::new(package_bytes.clone());

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

    let config_path = runtime_config_path();
    let config = actr_config::ConfigParser::from_runtime_file(&config_path, package_info, vec![])?;

    let _obs_guard = init_observability(&config.observability)?;
    info!("Package Runtime Echo client host starting");
    info!("Signaling server: {:?}", config.signaling_url);

    let hyper_data_dir = actr_config::user_config::resolve_hyper_data_dir()?;
    let trust: Arc<dyn TrustProvider> = if env::var("TRUST_MODE")
        .map(|v| v == "production")
        .unwrap_or(false)
    {
        let ais_endpoint =
            env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:8081/ais".to_string());
        let base_endpoint = ais_endpoint.trim_end_matches("/ais").to_string();
        Arc::new(RegistryTrust::new(base_endpoint))
    } else {
        Arc::new(StaticTrust::new(load_package_public_key()?).context("invalid pubkey")?)
    };

    let hyper = Hyper::new(HyperConfig::new(&hyper_data_dir, trust)).await?;

    let ais_endpoint =
        env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:8081/ais".to_string());

    let actr_ref = hyper
        .attach(&package, config)
        .await?
        .register(&ais_endpoint)
        .await?
        .start()
        .await?;

    println!("===== Package Runtime Echo Client =====");
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
            break;
        }

        if line.is_empty() {
            print!("> ");
            std::io::stdout().flush().unwrap();
            continue;
        }

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
                error!("Guest dispatch failed: {:?}", e);
                println!("\n[Error] {}", e);
            }
        }

        print!("> ");
        std::io::stdout().flush().unwrap();
    }

    actr_ref.shutdown();
    actr_ref.wait_for_shutdown().await;
    Ok(())
}
