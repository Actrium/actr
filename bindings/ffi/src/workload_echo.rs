use actr_framework::{Bytes, Context, MessageDispatcher, Workload};
use actr_protocol::{ActrError, ActrId, ActrType, ActorResult, RpcEnvelope};
use async_trait::async_trait;
use tokio::sync::Mutex;

const ECHO_SERVICE_MANUFACTURER: &str = "acme";
const ECHO_SERVICE_NAME: &str = "EchoService";
const ECHO_SERVICE_VERSION: &str = "1.0.0";

#[derive(Default)]
pub struct EchoProxyWorkload {
    cached_server_id: Mutex<Option<ActrId>>,
}

impl EchoProxyWorkload {
    fn target_type() -> ActrType {
        ActrType {
            manufacturer: ECHO_SERVICE_MANUFACTURER.to_string(),
            name: ECHO_SERVICE_NAME.to_string(),
            version: ECHO_SERVICE_VERSION.to_string(),
        }
    }

    async fn resolve_server<C: Context>(&self, ctx: &C) -> ActorResult<ActrId> {
        if let Some(id) = self.cached_server_id.lock().await.clone() {
            return Ok(id);
        }

        let id = ctx.discover_route_candidate(&Self::target_type()).await?;
        *self.cached_server_id.lock().await = Some(id.clone());
        Ok(id)
    }

    async fn clear_cached_server(&self) {
        *self.cached_server_id.lock().await = None;
    }
}

impl Workload for EchoProxyWorkload {
    type Dispatcher = EchoProxyDispatcher;
}

pub struct EchoProxyDispatcher;

#[async_trait]
impl MessageDispatcher for EchoProxyDispatcher {
    type Workload = EchoProxyWorkload;

    async fn dispatch<C: Context>(
        workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        let server_id = workload.resolve_server(ctx).await?;
        let payload = envelope.payload.unwrap_or_default();

        match ctx
            .call_raw(&server_id, &envelope.route_key, payload.clone())
            .await
        {
            Ok(response) => Ok(response),
            Err(original_error) => {
                workload.clear_cached_server().await;
                let fresh_server_id = workload.resolve_server(ctx).await?;
                ctx.call_raw(&fresh_server_id, &envelope.route_key, payload)
                    .await
                    .map_err(|retry_error| {
                        ActrError::Internal(format!(
                            "echo proxy retry failed: {retry_error} (original: {original_error})"
                        ))
                    })
            }
        }
    }
}