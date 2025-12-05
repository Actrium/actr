use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use actrix_common::storage::SqliteNonceStorage;
use nonce_auth::{CredentialBuilder, CredentialVerifier, NonceError, storage::NonceStorage};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status, transport::Server};

use supervit::{
    HealthCheckRequest, HealthCheckResponse, NonceCredential, RegisterNodeRequest,
    RegisterNodeResponse, ReportRequest, ReportResponse, SupervisorService,
    SupervisorServiceClient, SupervisorServiceServer,
};

const TEST_SHARED_SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Clone, Default)]
struct NodeState {
    last_report_at: Option<i64>,
}

#[derive(Clone)]
struct TestSupervisorService {
    shared_secret: Arc<Vec<u8>>,
    nonce_storage: Arc<dyn NonceStorage + Send + Sync>,
    max_clock_skew_secs: u64,
    next_report_interval_secs: i32,
    nodes: Arc<RwLock<std::collections::HashMap<String, NodeState>>>,
}

impl TestSupervisorService {
    fn new<N: NonceStorage + Send + Sync + 'static>(
        shared_secret: Vec<u8>,
        nonce_storage: N,
        max_clock_skew_secs: u64,
        next_report_interval_secs: i32,
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
            nodes: Arc::new(RwLock::new(std::collections::HashMap::new())),
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
}

fn build_registration_fingerprint(request: &RegisterNodeRequest) -> String {
    use sha2::{Digest, Sha256};

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

#[tonic::async_trait]
impl SupervisorService for TestSupervisorService {
    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeResponse>, Status> {
        let req = request.into_inner();
        let fingerprint = build_registration_fingerprint(&req);
        let payload = format!("register:{}:{}", req.node_id, fingerprint);

        self.verify_credential(&req.credential, payload).await?;

        let mut nodes = self.nodes.write().await;
        nodes.insert(req.node_id.clone(), NodeState::default());

        let response = RegisterNodeResponse {
            success: true,
            error_message: None,
            server_timestamp: chrono::Utc::now().timestamp(),
            heartbeat_interval_secs: 5,
            resource_version: Some(1),
            registered_at_iso: None,
        };

        Ok(Response::new(response))
    }

    async fn report(
        &self,
        request: Request<ReportRequest>,
    ) -> Result<Response<ReportResponse>, Status> {
        let req = request.into_inner();
        let payload = format!("report:{}:{}", req.node_id, req.timestamp);

        self.verify_credential(&req.credential, payload).await?;

        let mut nodes = self.nodes.write().await;
        let state = nodes.entry(req.node_id.clone()).or_default();
        state.last_report_at = Some(req.timestamp);

        let response = ReportResponse {
            received: true,
            server_timestamp: chrono::Utc::now().timestamp(),
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

        let response = HealthCheckResponse {
            healthy: true,
            server_timestamp: chrono::Utc::now().timestamp(),
            latency_ms: 1,
        };

        Ok(Response::new(response))
    }
}

async fn spawn_test_supervisor(
    shared_secret: Vec<u8>,
    max_clock_skew_secs: u64,
    next_report_interval_secs: i32,
) -> Result<(SocketAddr, TempDir, JoinHandle<()>), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let storage = SqliteNonceStorage::new_async(temp_dir.path()).await?;

    let service = TestSupervisorService::new(
        shared_secret,
        storage,
        max_clock_skew_secs,
        next_report_interval_secs,
    );

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let incoming = TcpListenerStream::new(listener);

    let handle = tokio::spawn(async move {
        Server::builder()
            .add_service(SupervisorServiceServer::new(service))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    Ok((addr, temp_dir, handle))
}

fn build_register_request(node_id: &str, shared_secret: &[u8]) -> RegisterNodeRequest {
    let request = RegisterNodeRequest {
        node_id: node_id.to_string(),
        name: "test-node".to_string(),
        location_tag: "test-location".to_string(),
        version: "0.0.1".to_string(),
        agent_addr: "127.0.0.1:60000".to_string(),
        credential: NonceCredential::default(),
        location: None,
        service_tags: vec![],
        power_reserve_level_init: Some(1),
        services: vec![],
    };

    let fingerprint = build_registration_fingerprint(&request);
    let payload = format!("register:{node_id}:{fingerprint}");
    let credential = CredentialBuilder::new(shared_secret)
        .sign(payload.as_bytes())
        .expect("credential generation should succeed");
    let credential = supervit::nonce_auth::to_proto_credential(credential);

    RegisterNodeRequest {
        credential,
        ..request
    }
}

fn build_report_request(node_id: &str, shared_secret: &[u8]) -> ReportRequest {
    let timestamp = chrono::Utc::now().timestamp();
    let payload = format!("report:{node_id}:{timestamp}");
    let credential = CredentialBuilder::new(shared_secret)
        .sign(payload.as_bytes())
        .expect("credential generation should succeed");
    let credential = supervit::nonce_auth::to_proto_credential(credential);

    ReportRequest {
        node_id: node_id.to_string(),
        timestamp,
        location_tag: "test-location".to_string(),
        version: "0.0.1".to_string(),
        name: "test-node".to_string(),
        power_reserve_level: 1,
        metrics: None,
        services: vec![],
        credential,
        realm_sync_version: 1,
    }
}

fn build_health_check_request(node_id: &str, shared_secret: &[u8]) -> HealthCheckRequest {
    let payload = format!("health_check:{node_id}");
    let credential = CredentialBuilder::new(shared_secret)
        .sign(payload.as_bytes())
        .expect("credential generation should succeed");
    let credential = supervit::nonce_auth::to_proto_credential(credential);

    HealthCheckRequest {
        node_id: node_id.to_string(),
        credential,
    }
}

#[tokio::test]
async fn register_report_health_flow_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let shared_secret = hex::decode(TEST_SHARED_SECRET)?;
    let (addr, _temp_dir, handle) = spawn_test_supervisor(shared_secret.clone(), 300, 15).await?;

    // Wait briefly for server to start listening
    tokio::time::sleep(Duration::from_millis(100)).await;

    let endpoint = format!("http://{addr}");
    let mut client = SupervisorServiceClient::connect(endpoint).await?;

    let node_id = "integration-node";

    let register_request = build_register_request(node_id, &shared_secret);
    let register_response = client.register_node(register_request).await?.into_inner();
    assert!(register_response.success);
    assert_eq!(register_response.heartbeat_interval_secs, 5);

    let report_request = build_report_request(node_id, &shared_secret);
    let report_response = client.report(report_request).await?.into_inner();
    assert!(report_response.received);
    assert_eq!(report_response.next_report_interval_secs, 15);

    let health_request = build_health_check_request(node_id, &shared_secret);
    let health_response = client.health_check(health_request).await?.into_inner();
    assert!(health_response.healthy);

    handle.abort();
    let _ = handle.await;

    Ok(())
}
