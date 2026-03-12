//! Client workload — forwards echo requests to the remote dynclib server

use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::echo::{EchoRequest, EchoResponse};
use actr_framework::{Context, Dest, MessageDispatcher, Workload};
use actr_protocol::{ActorResult, ActrError, ActrId, RpcEnvelope};
use async_trait::async_trait;
use prost::Message as ProstMessage;

/// Implement RpcRequest for type-safe calls
impl actr_protocol::RpcRequest for EchoRequest {
    type Response = EchoResponse;

    fn route_key() -> &'static str {
        "echo.EchoService.Echo"
    }

    fn payload_type() -> actr_protocol::PayloadType {
        actr_protocol::PayloadType::RpcReliable
    }
}

#[derive(Clone)]
pub struct ClientWorkload {
    pub server_id: Arc<Mutex<Option<ActrId>>>,
}

impl ClientWorkload {
    pub fn new() -> Self {
        Self {
            server_id: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_server_id(&self, server_id: ActrId) {
        *self.server_id.lock().await = Some(server_id);
    }
}

impl Workload for ClientWorkload {
    type Dispatcher = ClientDispatcher;
}

pub struct ClientDispatcher;

#[async_trait]
impl MessageDispatcher for ClientDispatcher {
    type Workload = ClientWorkload;

    async fn dispatch<C: Context>(
        workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        info!(
            "[ClientWorkload] Received request from App, route_key={}",
            envelope.route_key
        );

        let payload = envelope.payload.as_ref().ok_or_else(|| {
            ActrError::DecodeFailure("Missing payload in RpcEnvelope".to_string())
        })?;
        let request: EchoRequest =
            EchoRequest::decode(&**payload).map_err(|e| ActrError::DecodeFailure(e.to_string()))?;

        info!("[ClientWorkload] App message: {}", request.message);

        let server_id = workload.server_id.lock().await.clone();
        let server_id = match server_id {
            Some(id) => id,
            None => {
                error!("[ClientWorkload] Server ID not set");
                return Err(ActrError::Internal("Server ID not configured".to_string()));
            }
        };

        info!("[ClientWorkload] Forwarding to dynclib echo server...");

        // Call remote dynclib server via Dest::Actor
        let response: EchoResponse = ctx.call(&Dest::Actor(server_id), request).await?;

        info!(
            "[ClientWorkload] Got response from server: {}",
            response.reply
        );

        Ok(Bytes::from(response.encode_to_vec()))
    }
}
