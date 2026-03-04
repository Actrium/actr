//! Relay Client Workload - forwards media frames to actr-b

use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::generated::media_relay::{RelayFrameRequest, RelayFrameResponse};
use actr_framework::{Context, Dest, MessageDispatcher, Workload};
use actr_protocol::{ActrId, ActrType, RpcEnvelope};

/// Client Workload that forwards frames to remote actr-b
#[derive(Clone)]
pub struct RelayClientWorkload {
    pub server_id: Arc<Mutex<Option<ActrId>>>,
}

impl RelayClientWorkload {
    pub fn new() -> Self {
        Self {
            server_id: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_server_id(&self, server_id: ActrId) {
        *self.server_id.lock().await = Some(server_id);
    }
}

impl Workload for RelayClientWorkload {
    type Dispatcher = RelayClientDispatcher;
}

pub struct RelayClientDispatcher;

#[async_trait::async_trait]
impl MessageDispatcher for RelayClientDispatcher {
    type Workload = RelayClientWorkload;

    async fn dispatch<C: Context>(
        workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> actr_protocol::ActorResult<Bytes> {
        let payload = envelope.payload.as_ref().ok_or_else(|| {
            actr_protocol::ProtocolError::DecodeError("Missing payload in RpcEnvelope".to_string())
        })?;
        let request: RelayFrameRequest = prost::Message::decode(&**payload)
            .map_err(|e| actr_protocol::ProtocolError::SerializationError(e.to_string()))?;

        let frame = request.frame.as_ref().ok_or_else(|| {
            actr_protocol::ProtocolError::Actr(actr_protocol::ActrError::DecodeFailure {
                message: "MediaFrame is missing".to_string(),
            })
        })?;

        let server_id = workload.server_id.lock().await.clone().ok_or_else(|| {
            actr_protocol::ProtocolError::TransportError("Server ID not configured".to_string())
        })?;

        info!(
            "[RelayClientWorkload] Forwarding frame #{} ({} bytes) to actr-b via WebRTC P2P",
            frame.frame_number,
            frame.data.len()
        );

        // Call remote server via Dest::Actor
        let response: RelayFrameResponse = ctx.call(&Dest::Actor(server_id), request).await?;

        Ok(Bytes::from(prost::Message::encode_to_vec(&response)))
    }
}
