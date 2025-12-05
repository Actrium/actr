//! Minimal SupervisorService gRPC server for testing supervisord registration/report flows.
//!
//! This binary boots a simple SupervisorService implementation that accepts
//! RegisterNode, Report, and HealthCheck calls. It verifies nonce-auth
//! credentials with a shared secret, records basic node state in memory, and
//! responds with fixed intervals to drive client-side scheduling.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p supervit --bin supervisor_server -- --bind 127.0.0.1:50051
//! ```
//!
//! Optional flags:
//! - `--shared-secret <hex>`: Shared secret for nonce-auth (default matches other demos)
//! - `--data-dir <path>`: Directory for nonce storage (default: system temp)
//! - `--max-clock-skew-secs <u64>`: Allowed clock skew for credentials (default: 300s)
//! - `--report-interval-secs <i32>`: Interval suggested in ReportResponse
//! - `--heartbeat-interval-secs <i32>`: Interval suggested in RegisterNodeResponse

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actrix_common::storage::SqliteNonceStorage;
use chrono::Utc;
use clap::Parser;
use nonce_auth::{CredentialVerifier, NonceError, storage::NonceStorage};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status, transport::Server};
use tracing::{debug, info};

use supervit::{
    HealthCheckRequest, HealthCheckResponse, NonceCredential, RegisterNodeRequest,
    RegisterNodeResponse, ReportRequest, ReportResponse, SupervisorService,
    SupervisorServiceServer,
};

/// Default shared secret for testing (hex encoded, 32 bytes)
const DEFAULT_SHARED_SECRET: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Debug, Parser)]
#[command(name = "supervisor_server")]
#[command(about = "Simple SupervisorService gRPC server for supervisord testing")]
struct Args {
    /// Bind address for gRPC server (host:port)
    #[arg(long, default_value = "127.0.0.1:50051")]
    bind: String,

    /// Shared secret (hex encoded)
    #[arg(long, default_value = DEFAULT_SHARED_SECRET)]
    shared_secret: String,

    /// Directory for nonce storage (sqlite). Defaults to system temp dir.
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Allowed clock skew for credential verification (seconds)
    #[arg(long, default_value_t = 300)]
    max_clock_skew_secs: u64,

    /// Next report interval returned to clients (seconds)
    #[arg(long, default_value_t = 30)]
    report_interval_secs: i32,

    /// Heartbeat interval returned in RegisterNodeResponse (seconds)
    #[arg(long, default_value_t = 15)]
    heartbeat_interval_secs: i32,
}

#[derive(Clone, Debug, Default)]
struct NodeState {
    name: String,
    location_tag: String,
    agent_addr: String,
    version: String,
    last_report_at: Option<i64>,
}

#[derive(Clone)]
struct SupervisorServiceImpl {
    shared_secret: Arc<Vec<u8>>,
    nonce_storage: Arc<dyn NonceStorage + Send + Sync>,
    max_clock_skew_secs: u64,
    next_report_interval_secs: i32,
    heartbeat_interval_secs: i32,
    nodes: Arc<RwLock<HashMap<String, NodeState>>>,
}

impl SupervisorServiceImpl {
    fn new<N: NonceStorage + Send + Sync + 'static>(
        shared_secret: Vec<u8>,
        nonce_storage: N,
        max_clock_skew_secs: u64,
        next_report_interval_secs: i32,
        heartbeat_interval_secs: i32,
    ) -> Self {
        Self {
            shared_secret: Arc::new(shared_secret),
            nonce_storage: Arc::new(nonce_storage),
            max_clock_skew_secs: if max_clock_skew_secs == 0 {
                300
            } else {
                max_clock_skew_secs
            },
            next_report_interval_secs,
            heartbeat_interval_secs,
            nodes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn verify_credential(
        &self,
        credential: &NonceCredential,
        payload: String,
    ) -> Result<(), Status> {
        let nonce_credential = nonce_auth::NonceCredential {
            timestamp: credential.timestamp,
            nonce: credential.nonce.clone(),
            signature: credential.signature.clone(),
        };

        CredentialVerifier::new(self.nonce_storage.clone())
            .with_secret(&self.shared_secret)
            .with_time_window(Duration::from_secs(self.max_clock_skew_secs))
            .with_storage_ttl(Duration::from_secs(self.max_clock_skew_secs + 300))
            .verify(&nonce_credential, payload.as_bytes())
            .await
            .map_err(|e| match e {
                NonceError::DuplicateNonce => Status::unauthenticated("duplicate nonce"),
                NonceError::TimestampOutOfWindow => {
                    Status::unauthenticated("timestamp outside allowed window")
                }
                NonceError::InvalidSignature => Status::unauthenticated("invalid signature"),
                _ => Status::internal(format!("credential verification failed: {e}")),
            })
    }

    fn build_registration_fingerprint(request: &RegisterNodeRequest) -> String {
        let mut tags = request.service_tags.clone();
        tags.sort();
        tags.dedup();

        let mut service_entries = request
            .services
            .iter()
            .map(|svc| {
                let mut svc_tags = svc.tags.clone();
                svc_tags.sort();
                format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    svc.name,
                    svc.r#type,
                    svc.domain_name,
                    svc.port_info,
                    svc.status,
                    svc.url.clone().unwrap_or_default(),
                    svc_tags.join(",")
                )
            })
            .collect::<Vec<_>>();
        service_entries.sort();

        let location = request.location.clone().unwrap_or_default();
        let power_level = request.power_reserve_level_init.unwrap_or(0);

        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            request.node_id,
            request.agent_addr,
            request.location_tag,
            location,
            power_level,
            tags.join(","),
            service_entries.join(";"),
        );

        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[tonic::async_trait]
impl SupervisorService for SupervisorServiceImpl {
    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeResponse>, Status> {
        let req = request.into_inner();

        // Print full registration request details
        info!("=== RegisterNode Request ===");
        info!("node_id: {}", req.node_id);
        info!("name: {}", req.name);
        info!("location_tag: {}", req.location_tag);
        info!("version: {}", req.version);
        info!("agent_addr: {}", req.agent_addr);
        info!("location: {:?}", req.location);
        info!("service_tags: {:?}", req.service_tags);
        info!(
            "power_reserve_level_init: {:?}",
            req.power_reserve_level_init
        );
        info!("services count: {}", req.services.len());
        for (idx, svc) in req.services.iter().enumerate() {
            info!(
                "  service[{}]: name={}, type={}, domain={}, port={}, status={}, url={:?}, tags={:?}",
                idx,
                svc.name,
                svc.r#type,
                svc.domain_name,
                svc.port_info,
                svc.status,
                svc.url,
                svc.tags
            );
        }
        info!(
            "credential: timestamp={}, nonce={}",
            req.credential.timestamp, req.credential.nonce
        );
        info!("============================");

        let fingerprint = Self::build_registration_fingerprint(&req);
        let payload = format!("register:{}:{}", req.node_id, fingerprint);

        self.verify_credential(&req.credential, payload).await?;

        let now = Utc::now();
        let mut nodes = self.nodes.write().await;
        nodes.insert(
            req.node_id.clone(),
            NodeState {
                name: req.name.clone(),
                location_tag: req.location_tag.clone(),
                agent_addr: req.agent_addr.clone(),
                version: req.version.clone(),
                last_report_at: None,
            },
        );

        info!(
            node_id = %req.node_id,
            version = %req.version,
            agent_addr = %req.agent_addr,
            services = req.services.len(),
            "node registered"
        );

        // Print detailed service information
        for (idx, svc) in req.services.iter().enumerate() {
            info!(
                "  service[{}]: name={}, type={}, domain={}, port={}, status={}, url={:?}, tags={:?}",
                idx,
                svc.name,
                svc.r#type,
                svc.domain_name,
                svc.port_info,
                svc.status,
                svc.url,
                svc.tags
            );
        }

        let response = RegisterNodeResponse {
            success: true,
            error_message: None,
            server_timestamp: now.timestamp(),
            heartbeat_interval_secs: self.heartbeat_interval_secs,
            resource_version: Some(1),
            registered_at_iso: Some(now.to_rfc3339()),
        };

        Ok(Response::new(response))
    }

    async fn report(
        &self,
        request: Request<ReportRequest>,
    ) -> Result<Response<ReportResponse>, Status> {
        let req = request.into_inner();

        // Print full report request details
        debug!("=== Report Request ===");
        debug!("node_id: {}", req.node_id);
        debug!("timestamp: {}", req.timestamp);
        debug!("name: {}", req.name);
        debug!("location_tag: {}", req.location_tag);
        debug!("version: {}", req.version);
        debug!("power_reserve_level: {}", req.power_reserve_level);
        debug!("realm_sync_version: {}", req.realm_sync_version);
        debug!("services count: {}", req.services.len());
        for (idx, svc) in req.services.iter().enumerate() {
            debug!(
                "  service[{}]: name={}, type={}, healthy={}, connections={}, requests={}, failed={}, latency={:.2}ms",
                idx,
                svc.name,
                svc.r#type,
                svc.is_healthy,
                svc.active_connections,
                svc.total_requests,
                svc.failed_requests,
                svc.average_latency_ms
            );
        }
        if let Some(ref metrics) = req.metrics {
            debug!(
                "metrics: cpu={:.2}%, mem={:.2}%, net_rx={}, net_tx={}",
                metrics.cpu_usage_percent,
                metrics.memory_usage_percent,
                metrics.network_rx_bytes,
                metrics.network_tx_bytes
            );
        }
        debug!(
            "credential: timestamp={}, nonce={}",
            req.credential.timestamp, req.credential.nonce
        );
        debug!("=====================");

        let payload = format!("report:{}:{}", req.node_id, req.timestamp);

        self.verify_credential(&req.credential, payload).await?;

        let now = Utc::now();
        let mut nodes = self.nodes.write().await;
        let state = nodes
            .entry(req.node_id.clone())
            .or_insert_with(|| NodeState {
                name: req.name.clone(),
                location_tag: req.location_tag.clone(),
                agent_addr: String::new(),
                version: req.version.clone(),
                last_report_at: None,
            });
        state.last_report_at = Some(req.timestamp);
        state.version = req.version.clone();
        state.location_tag = req.location_tag.clone();

        let metrics_summary = req
            .metrics
            .as_ref()
            .map(|m| {
                format!(
                    "cpu: {:.2}%, mem: {:.2}%",
                    m.cpu_usage_percent, m.memory_usage_percent
                )
            })
            .unwrap_or_else(|| "metrics unavailable".to_string());

        info!(
            node_id = %req.node_id,
            name = %state.name,
            agent_addr = %state.agent_addr,
            power_level = req.power_reserve_level,
            services = req.services.len(),
            realm_sync_version = req.realm_sync_version,
            %metrics_summary,
            "report received"
        );

        let response = ReportResponse {
            received: true,
            server_timestamp: now.timestamp(),
            next_report_interval_secs: self.next_report_interval_secs,
            directive: None,
        };

        Ok(Response::new(response))
    }

    async fn health_check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let req = request.into_inner();
        let payload = format!("health_check:{}", req.node_id);

        self.verify_credential(&req.credential, payload).await?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));

        debug!(node_id = %req.node_id, "health check accepted");

        let response = HealthCheckResponse {
            healthy: true,
            server_timestamp: now.as_secs() as i64,
            latency_ms: 1,
        };

        Ok(Response::new(response))
    }
}

fn default_data_dir() -> PathBuf {
    std::env::temp_dir().join("supervit_supervisor_server")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(true)
        .init();

    let data_dir = args.data_dir.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    info!("Using data dir for nonce storage: {:?}", data_dir);

    let shared_secret =
        hex::decode(&args.shared_secret).map_err(|e| format!("invalid shared secret hex: {e}"))?;

    let nonce_storage = SqliteNonceStorage::new_async(&data_dir).await?;

    let service = SupervisorServiceImpl::new(
        shared_secret,
        nonce_storage,
        args.max_clock_skew_secs,
        args.report_interval_secs,
        args.heartbeat_interval_secs,
    );

    let addr: SocketAddr = args.bind.parse()?;
    info!("SupervisorService listening on {}", addr);
    info!("Next report interval: {}s", args.report_interval_secs);
    info!("Heartbeat interval: {}s", args.heartbeat_interval_secs);

    Server::builder()
        .add_service(SupervisorServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
