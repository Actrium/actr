use actrix_proto::{
    CreateTenantRequest, CreateTenantResponse, DeleteTenantRequest, DeleteTenantResponse,
    GetConfigRequest, GetConfigResponse, GetNodeInfoRequest, GetNodeInfoResponse, GetTenantRequest,
    GetTenantResponse, ListTenantsRequest, ListTenantsResponse, NonceCredential, ShutdownRequest,
    ShutdownResponse, SupervisedService, UpdateConfigRequest, UpdateConfigResponse,
    UpdateTenantRequest, UpdateTenantResponse,
};
use nonce_auth::{CredentialVerifier, NonceError, storage::NonceStorage};
use std::sync::Arc;
use std::time::Duration;
use tonic::{Request, Response, Status};

/// 请求体需要提供认证载荷与凭证
pub trait CredentialPayload {
    fn credential(&self) -> &NonceCredential;
    fn auth_payload(&self, node_id: &str) -> String;
}

#[derive(Clone)]
struct VerifierState {
    node_id: String,
    shared_secret: Arc<Vec<u8>>,
    nonce_storage: Arc<dyn NonceStorage + Send + Sync>,
    max_clock_skew_secs: u64,
}

impl VerifierState {
    async fn verify(&self, credential: &NonceCredential, payload: String) -> Result<(), Status> {
        let nonce_credential = nonce_auth::NonceCredential {
            timestamp: credential.timestamp,
            nonce: credential.nonce.clone(),
            signature: credential.signature.clone(),
        };

        let verifier = CredentialVerifier::new(self.nonce_storage.clone())
            .with_secret(&self.shared_secret)
            .with_time_window(Duration::from_secs(self.max_clock_skew_secs))
            .with_storage_ttl(Duration::from_secs(self.max_clock_skew_secs + 300));

        verifier
            .verify(&nonce_credential, payload.as_bytes())
            .await
            .map_err(|e| map_nonce_error(e, "credential verification failed"))
    }
}

/// 在进入业务实现前统一做 NonceCredential 校验的包装服务
#[derive(Clone)]
pub struct AuthService<S> {
    inner: S,
    verifier: Arc<VerifierState>,
}

impl<S> AuthService<S> {
    pub fn new(
        inner: S,
        node_id: impl Into<String>,
        shared_secret: Arc<Vec<u8>>,
        nonce_storage: Arc<dyn NonceStorage + Send + Sync>,
        max_clock_skew_secs: u64,
    ) -> Self {
        let time_window = if max_clock_skew_secs == 0 {
            300
        } else {
            max_clock_skew_secs
        };

        Self {
            inner,
            verifier: Arc::new(VerifierState {
                node_id: node_id.into(),
                shared_secret,
                nonce_storage,
                max_clock_skew_secs: time_window,
            }),
        }
    }

    async fn verify_body<T: CredentialPayload>(&self, body: &T) -> Result<(), Status> {
        let payload = body.auth_payload(&self.verifier.node_id);
        self.verifier.verify(body.credential(), payload).await
    }
}

#[tonic::async_trait]
impl<S> SupervisedService for AuthService<S>
where
    S: SupervisedService + Send + Sync + Clone + 'static,
{
    async fn update_config(
        &self,
        request: Request<UpdateConfigRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.update_config(request).await
    }

    async fn get_config(
        &self,
        request: Request<GetConfigRequest>,
    ) -> Result<Response<GetConfigResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.get_config(request).await
    }

    async fn create_tenant(
        &self,
        request: Request<CreateTenantRequest>,
    ) -> Result<Response<CreateTenantResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.create_tenant(request).await
    }

    async fn get_tenant(
        &self,
        request: Request<GetTenantRequest>,
    ) -> Result<Response<GetTenantResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.get_tenant(request).await
    }

    async fn update_tenant(
        &self,
        request: Request<UpdateTenantRequest>,
    ) -> Result<Response<UpdateTenantResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.update_tenant(request).await
    }

    async fn delete_tenant(
        &self,
        request: Request<DeleteTenantRequest>,
    ) -> Result<Response<DeleteTenantResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.delete_tenant(request).await
    }

    async fn list_tenants(
        &self,
        request: Request<ListTenantsRequest>,
    ) -> Result<Response<ListTenantsResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.list_tenants(request).await
    }

    async fn get_node_info(
        &self,
        request: Request<GetNodeInfoRequest>,
    ) -> Result<Response<GetNodeInfoResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.get_node_info(request).await
    }

    async fn shutdown(
        &self,
        request: Request<ShutdownRequest>,
    ) -> Result<Response<ShutdownResponse>, Status> {
        self.verify_body(request.get_ref()).await?;
        self.inner.shutdown(request).await
    }
}

// ========= 请求类型的载荷构造实现 =========

impl CredentialPayload for UpdateConfigRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!(
            "update_config:{node_id}:{}:{}",
            self.config_type, self.config_key
        )
    }
}

impl CredentialPayload for GetConfigRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!(
            "get_config:{node_id}:{}:{}",
            self.config_type, self.config_key
        )
    }
}

impl CredentialPayload for CreateTenantRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!("create_realm:{node_id}:{}", self.tenant_id)
    }
}

impl CredentialPayload for GetTenantRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!("get_realm:{node_id}:{}", self.tenant_id)
    }
}

impl CredentialPayload for UpdateTenantRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!("update_realm:{node_id}:{}", self.tenant_id)
    }
}

impl CredentialPayload for DeleteTenantRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!("delete_realm:{node_id}:{}", self.tenant_id)
    }
}

impl CredentialPayload for ListTenantsRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!("list_realms:{node_id}")
    }
}

impl CredentialPayload for GetNodeInfoRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!("node_info:{node_id}")
    }
}

impl CredentialPayload for ShutdownRequest {
    fn credential(&self) -> &NonceCredential {
        &self.credential
    }

    fn auth_payload(&self, node_id: &str) -> String {
        format!("shutdown:{node_id}")
    }
}

fn map_nonce_error(err: NonceError, context: &str) -> Status {
    match err {
        NonceError::DuplicateNonce => {
            Status::unauthenticated(format!("{context}: nonce already used"))
        }
        NonceError::TimestampOutOfWindow => {
            Status::unauthenticated(format!("{context}: timestamp out of range"))
        }
        NonceError::InvalidSignature => {
            Status::unauthenticated(format!("{context}: invalid signature"))
        }
        other => Status::internal(format!("{context}: {other}")),
    }
}
