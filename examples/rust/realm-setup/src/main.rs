//! realm-setup - CLI tool to setup realms in actrix for actr-examples
//!
//! This tool reads realm IDs from actr.toml configuration files and
//! creates them in actrix via the SupervisedService gRPC API.

mod generated {
    tonic::include_proto!("supervisor.v1");
}

use anyhow::{Context, Result};
use clap::Parser;
use generated::{
    CreateRealmRequest, GetRealmRequest, NonceCredential, ResourceType,
    supervised_service_client::SupervisedServiceClient,
};
use nonce_auth::CredentialBuilder;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, error, info, warn};

/// CLI arguments
#[derive(Parser, Debug)]
#[command(name = "realm-setup")]
#[command(about = "Setup realms in actrix from actr.toml configuration files")]
#[command(version)]
struct Args {
    /// Path to actrix-config.toml file
    #[arg(short = 'c', long, default_value = "actrix-config.toml")]
    actrix_config: PathBuf,

    /// Paths to actr.toml files to read realm IDs from
    #[arg(short = 'a', long = "actr-toml", required = true, num_args = 1..)]
    actr_tomls: Vec<PathBuf>,

    /// Endpoint of the actrix supervisord gRPC service
    #[arg(short = 'e', long, default_value = "http://127.0.0.1:50055")]
    endpoint: String,

    /// Node ID for authentication
    #[arg(short = 'n', long)]
    node_id: Option<String>,

    /// Shared secret (hex encoded) for authentication
    #[arg(short = 's', long)]
    shared_secret: Option<String>,

    /// Request timeout in seconds
    #[arg(short = 't', long, default_value = "30")]
    timeout: u64,

    /// Enable verbose output
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Realm expiration time (Unix timestamp). Default is 10 years from now.
    #[arg(long)]
    expires_at: Option<u64>,
}

/// Partial actrix-config.toml structure
#[derive(Debug, Deserialize)]
struct ActrixConfig {
    supervisor: Option<SupervisorSection>,
}

#[derive(Debug, Deserialize)]
struct SupervisorSection {
    supervisord: Option<SupervisordConfig>,
    client: Option<ClientConfig>,
}

#[derive(Debug, Deserialize)]
struct SupervisordConfig {
    ip: Option<String>,
    port: Option<u16>,
    advertised_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClientConfig {
    node_id: Option<String>,
    shared_secret: Option<String>,
}

/// Partial actr.toml structure
#[derive(Debug, Deserialize)]
struct ActrConfig {
    system: Option<SystemSection>,
}

#[derive(Debug, Deserialize)]
struct SystemSection {
    deployment: Option<DeploymentSection>,
}

#[derive(Debug, Deserialize)]
struct DeploymentSection {
    #[serde(alias = "realm")]
    realm_id: Option<u32>,
}

/// Build payload string for credential signing
fn build_payload(action: &str, node_id: &str, subject: Option<&str>) -> String {
    if let Some(target) = subject {
        format!("{action}:{node_id}:{target}")
    } else {
        format!("{action}:{node_id}")
    }
}

/// Create a nonce credential for authentication
fn create_credential(shared_secret: &[u8], payload: &str) -> Result<NonceCredential> {
    let credential = CredentialBuilder::new(shared_secret)
        .sign(payload.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to create credential: {}", e))?;

    Ok(NonceCredential {
        timestamp: credential.timestamp,
        nonce: credential.nonce,
        signature: credential.signature,
    })
}

/// Parse realm IDs from actr.toml files
fn parse_realm_ids(actr_tomls: &[PathBuf]) -> Result<HashSet<u32>> {
    let mut realm_ids = HashSet::new();

    for path in actr_tomls {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read actr.toml: {}", path.display()))?;

        let config: ActrConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse actr.toml: {}", path.display()))?;

        if let Some(system) = config.system {
            if let Some(deployment) = system.deployment {
                if let Some(realm) = deployment.realm_id {
                    info!("Found realm {} in {}", realm, path.display());
                    realm_ids.insert(realm);
                }
            }
        }
    }

    Ok(realm_ids)
}

/// Parse actrix-config.toml for supervisor configuration
fn parse_actrix_config(path: &PathBuf) -> Result<(String, String, String)> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read actrix-config.toml: {}", path.display()))?;

    let config: ActrixConfig = toml::from_str(&content)
        .with_context(|| format!("Failed to parse actrix-config.toml: {}", path.display()))?;

    let supervisor = config
        .supervisor
        .ok_or_else(|| anyhow::anyhow!("Missing [supervisor] section in actrix-config.toml"))?;

    let supervisord = supervisor.supervisord.unwrap_or(SupervisordConfig {
        ip: None,
        port: None,
        advertised_ip: None,
    });

    let client = supervisor
        .client
        .ok_or_else(|| anyhow::anyhow!("Missing [supervisor.client] section"))?;

    let ip = supervisord
        .advertised_ip
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = supervisord.port.unwrap_or(50055);
    let endpoint = format!("http://{}:{}", ip, port);

    let node_id = client
        .node_id
        .ok_or_else(|| anyhow::anyhow!("Missing node_id in [supervisor.client] section"))?;

    let shared_secret = client
        .shared_secret
        .ok_or_else(|| anyhow::anyhow!("Missing shared_secret in [supervisor.client] section"))?;

    Ok((endpoint, node_id, shared_secret))
}

/// Check if a realm already exists
async fn realm_exists(
    client: &mut SupervisedServiceClient<Channel>,
    realm_id: u32,
    node_id: &str,
    shared_secret: &[u8],
) -> Result<bool> {
    let payload = build_payload("get_realm", node_id, Some(&realm_id.to_string()));
    let credential = create_credential(shared_secret, &payload)?;

    let request = tonic::Request::new(GetRealmRequest {
        realm_id,
        credential,
    });

    match client.get_realm(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            Ok(resp.success && resp.realm.is_some())
        }
        Err(status) => {
            debug!("GetRealm returned error (likely not found): {}", status);
            Ok(false)
        }
    }
}

/// Create a realm
async fn create_realm(
    client: &mut SupervisedServiceClient<Channel>,
    realm_id: u32,
    node_id: &str,
    shared_secret: &[u8],
    expires_at: u64,
) -> Result<bool> {
    let payload = build_payload("create_realm", node_id, Some(&realm_id.to_string()));
    let credential = create_credential(shared_secret, &payload)?;

    // Allow all server types for example realms
    let use_servers = vec![
        ResourceType::Stun as i32,
        ResourceType::Turn as i32,
        ResourceType::Signaling as i32,
        ResourceType::Ais as i32,
        ResourceType::Ks as i32,
    ];

    let request = tonic::Request::new(CreateRealmRequest {
        realm_id,
        name: format!("example-realm-{}", realm_id),
        enabled: true,
        use_servers,
        credential,
        version: 1,
        expires_at,
    });

    let response = client
        .create_realm(request)
        .await
        .map_err(|e| anyhow::anyhow!("gRPC CreateRealm failed: {}", e))?;

    let resp = response.into_inner();

    if resp.success {
        info!("Successfully created realm {}", realm_id);
        Ok(true)
    } else {
        let err_msg = resp.error_message.unwrap_or_default();
        warn!("Failed to create realm {}: {}", realm_id, err_msg);
        Ok(false)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let filter = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // Parse realm IDs from actr.toml files
    info!(
        "Parsing realm IDs from {} actr.toml files...",
        args.actr_tomls.len()
    );
    let realm_ids = parse_realm_ids(&args.actr_tomls)?;

    if realm_ids.is_empty() {
        warn!("No realm IDs found in the provided actr.toml files");
        return Ok(());
    }

    info!(
        "Found {} unique realm ID(s): {:?}",
        realm_ids.len(),
        realm_ids
    );

    // Determine endpoint, node_id, and shared_secret
    let (endpoint, node_id, shared_secret_hex) =
        if args.node_id.is_some() && args.shared_secret.is_some() {
            // Use CLI arguments
            (
                args.endpoint.clone(),
                args.node_id.clone().unwrap(),
                args.shared_secret.clone().unwrap(),
            )
        } else {
            // Read from actrix-config.toml
            info!(
                "Reading configuration from {}",
                args.actrix_config.display()
            );
            let (cfg_endpoint, cfg_node_id, cfg_shared_secret) =
                parse_actrix_config(&args.actrix_config)?;

            (
                if args.endpoint != "http://127.0.0.1:50055" {
                    args.endpoint.clone()
                } else {
                    cfg_endpoint
                },
                args.node_id.unwrap_or(cfg_node_id),
                args.shared_secret.unwrap_or(cfg_shared_secret),
            )
        };

    // Decode shared secret
    let shared_secret =
        hex::decode(&shared_secret_hex).with_context(|| "Failed to decode shared_secret as hex")?;

    info!("Connecting to actrix supervisord at {}", endpoint);
    debug!("Using node_id: {}", node_id);

    // Connect to supervisord
    let ep = Endpoint::from_shared(endpoint.clone())
        .map_err(|e| anyhow::anyhow!("Invalid endpoint: {}", e))?
        .timeout(Duration::from_secs(args.timeout))
        .connect_timeout(Duration::from_secs(args.timeout));

    let channel = ep
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to actrix supervisord: {}", e))?;

    let mut client = SupervisedServiceClient::new(channel);

    // Calculate default expires_at (10 years from now)
    let expires_at = args.expires_at.unwrap_or_else(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now + 10 * 365 * 24 * 60 * 60 // 10 years
    });

    // Create realms
    let mut success_count = 0;
    let mut skip_count = 0;
    let mut fail_count = 0;

    for realm_id in realm_ids {
        // Check if realm already exists
        if realm_exists(&mut client, realm_id, &node_id, &shared_secret).await? {
            info!("Realm {} already exists, skipping", realm_id);
            skip_count += 1;
            continue;
        }

        // Create the realm
        if create_realm(&mut client, realm_id, &node_id, &shared_secret, expires_at).await? {
            success_count += 1;
        } else {
            fail_count += 1;
        }
    }

    // Summary
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("Realm setup complete:");
    info!("  Created: {}", success_count);
    info!("  Skipped (already exists): {}", skip_count);
    info!("  Failed: {}", fail_count);
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    if fail_count > 0 {
        error!("Some realms failed to create");
        std::process::exit(1);
    }

    Ok(())
}
